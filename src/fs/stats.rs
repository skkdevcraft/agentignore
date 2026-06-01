//! Live stats collector for the AgentIgnore mount dashboard.
//!
//! Collects real-time operation counters, path access tracking, and handle
//! accounting.  The collector is purely additive — no stats are ever
//! decremented (except tick counters which reset each snapshot).
//!
//! # Design
//!
//! - All operation counters use `AtomicU64` for lock-free reads/writes.
//! - Recent paths use a `Mutex<VecDeque>` because the path list is small (< 10
//!   entries) and updates are infrequent relative to FUSE method calls.
//! - Open handles use `AtomicIsize` for cheap inc/dec.
//! - The collector is 100 % no-op when `Option<Arc<StatsCollector>>` is `None`:
//!   checking a single `Option` is a single branch that compiles to a compare
//!   and a conditional jump.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── Operation types ─────────────────────────────────────────────────────────

/// Every FUSE operation that is tracked independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpType {
    Lookup,
    Getattr,
    Readdir,
    Open,
    Read,
    Write,
    Release,
    Readlink,
    Create,
    Mkdir,
    Unlink,
    Rmdir,
    Rename,
    Link,
    Symlink,
    Statfs,
    Setattr,
    Flush,
    Fsync,
    Fsyncdir,
    Access,
    /// Not a FUSE op — synthetic counter for denied access events.
    Denied,
}

impl OpType {
    /// Human-readable label (padded for alignment in the dashboard).
    pub fn label(&self) -> &'static str {
        match self {
            Self::Lookup => "LOOKUP",
            Self::Getattr => "GETATTR",
            Self::Readdir => "READDIR",
            Self::Open => "OPEN",
            Self::Read => "READ",
            Self::Write => "WRITE",
            Self::Release => "RELEASE",
            Self::Readlink => "READLINK",
            Self::Setattr => "SETATTR",
            Self::Flush => "FLUSH",
            Self::Fsync => "FSYNC",
            Self::Fsyncdir => "FSYNCDIR",
            Self::Access => "ACCESS",
            Self::Create => "CREATE",
            Self::Mkdir => "MKDIR",
            Self::Unlink => "UNLINK",
            Self::Rmdir => "RMDIR",
            Self::Rename => "RENAME",
            Self::Link => "LINK",
            Self::Symlink => "SYMLINK",
            Self::Statfs => "STATFS",
            Self::Denied => "DENIED",
        }
    }

    /// All tracked operation types (used for iteration).
    pub const ALL: &'static [OpType] = &[
        Self::Lookup,
        Self::Getattr,
        Self::Readdir,
        Self::Open,
        Self::Read,
        Self::Write,
        Self::Release,
        Self::Readlink,
        Self::Setattr,
        Self::Flush,
        Self::Fsync,
        Self::Fsyncdir,
        Self::Access,
        Self::Create,
        Self::Mkdir,
        Self::Unlink,
        Self::Rmdir,
        Self::Rename,
        Self::Link,
        Self::Symlink,
        Self::Statfs,
        Self::Denied,
    ];
}

// ── Access kind ─────────────────────────────────────────────────────────────

/// Whether an accessed file was visible, hidden by policy, or bypass-hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// Visible to the requesting process — no policy match.
    Allowed,
    /// Hidden by `.agentignore` (returned ENOENT).
    Denied,
    /// Hidden by `.agentignore` but bypassed via `.agentallow`.
    Bypassed,
}

// ── Per-path entry (internal) ───────────────────────────────────────────────

#[derive(Debug, Clone)]
struct PathEntry {
    path: PathBuf,
    pid: u32,
    process_name: String,
    access: AccessKind,
    hit_count: usize,
    last_op: OpType,
}

// ── Snapshot types (public) ─────────────────────────────────────────────────

/// A point-in-time snapshot of the stats collector.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Per-op-type counters: (total_since_mount, tick_count_since_last_snapshot).
    pub ops: BTreeMap<OpType, (u64, u64)>,
    /// Most-recently-touched unique paths (max 10, newest first).
    pub recent_paths: Vec<PathSnapshot>,
    /// Number of currently open file handles.
    pub open_handles: isize,
    /// Wall-clock time since the stats collector was created.
    pub uptime: Duration,
    /// The mounted source directory.
    pub source: PathBuf,
    /// The mountpoint path.
    pub mountpoint: PathBuf,
}

/// Info about one recently accessed path.
#[derive(Debug, Clone)]
pub struct PathSnapshot {
    pub path: PathBuf,
    pub pid: u32,
    pub process_name: String,
    pub access: AccessKind,
    pub hit_count: usize,
    pub last_op: OpType,
}

// ── StatsCollector ──────────────────────────────────────────────────────────

/// Lock-free-ish stats collector for the mount dashboard.
///
/// Thread-safe by construction.  Clone the `Arc` to share across threads.
pub struct StatsCollector {
    /// Per-op total counters.
    op_totals: [AtomicU64; 22],
    /// Per-op tick counters (reset on each `snapshot()`).
    op_ticks: [AtomicU64; 22],
    /// Recent unique paths (newest first, max 10).
    recent: Mutex<VecDeque<PathEntry>>,
    /// Current open handle count.
    open_handles: AtomicIsize,
    /// Creation timestamp for uptime calculation.
    created_at: Instant,
    /// Mount source path (set on construction via builder or after creation).
    source: Mutex<PathBuf>,
    /// Mountpoint path (set on construction via builder or after creation).
    mountpoint: Mutex<PathBuf>,
}

use std::collections::VecDeque;

impl StatsCollector {
    /// Create a new stats collector.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            op_totals: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            op_ticks: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            recent: Mutex::new(VecDeque::with_capacity(11)),
            open_handles: AtomicIsize::new(0),
            created_at: Instant::now(),
            source: Mutex::new(PathBuf::new()),
            mountpoint: Mutex::new(PathBuf::new()),
        })
    }

    /// Set the source path metadata.
    pub fn set_source(&self, source: PathBuf) {
        *self.source.lock().unwrap() = source;
    }

    /// Set the mountpoint path metadata.
    pub fn set_mountpoint(&self, mountpoint: PathBuf) {
        *self.mountpoint.lock().unwrap() = mountpoint;
    }

    /// Return the op index for an `OpType`.
    fn op_idx(op: OpType) -> usize {
        op as usize
    }

    /// Record a filesystem operation.
    ///
    /// Increments both the permanent total and the tick counter.
    /// Updates the recent-paths list with the given path info.
    ///
    /// # Canonicalization
    ///
    /// Callers are expected to pass a canonical path.  Every FUSE call site
    /// already computes the canonical path for policy checks, so re-
    /// canonicalizing here would be a redundant syscall.  The only exception
    /// (`lookup`) has been fixed to pass the canonical child path.
    pub fn record_op(&self, op: OpType, path: &std::path::Path, pid: u32, kind: AccessKind) {
        let idx = Self::op_idx(op);
        self.op_totals[idx].fetch_add(1, Ordering::Relaxed);
        self.op_ticks[idx].fetch_add(1, Ordering::Relaxed);

        // Update recent paths
        let mut recent = self.recent.lock().unwrap();

        // Try to find existing entry for this path and take ownership
        // directly via remove() instead of cloning and re-inserting.
        if let Some(pos) = recent.iter().position(|e| e.path == path) {
            let mut entry = recent.remove(pos).unwrap();
            entry.hit_count += 1;
            entry.access = kind;
            entry.pid = pid;
            entry.last_op = op;
            recent.push_front(entry);
        } else {
            // New unique path — insert at front
            recent.push_front(PathEntry {
                path: path.to_path_buf(),
                pid,
                process_name: crate::tools::get_process_name_cached(pid)
                    .unwrap_or_else(|| "<unknown>".to_string()),
                access: kind,
                hit_count: 1,
                last_op: op,
            });
            // Keep max 10 entries
            while recent.len() > 10 {
                recent.pop_back();
            }
        }
    }

    /// Record an open handle.
    pub fn record_handle_open(&self) {
        self.open_handles.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a closed handle.
    pub fn record_handle_close(&self) {
        self.open_handles.fetch_sub(1, Ordering::Relaxed);
    }

    /// Take a snapshot and reset tick counters.
    pub fn snapshot(&self, clear: bool) -> Snapshot {
        let mut ops = BTreeMap::new();

        for op_type in OpType::ALL {
            let idx = Self::op_idx(*op_type);
            let total = self.op_totals[idx].load(Ordering::Relaxed);
            let ticks = self.op_ticks[idx].swap(0, Ordering::Relaxed);
            ops.insert(*op_type, (total, ticks));
        }

        let recent = self.recent.lock().unwrap();
        let recent_paths: Vec<PathSnapshot> = recent
            .iter()
            .map(|e| PathSnapshot {
                path: e.path.clone(),
                pid: e.pid,
                process_name: e.process_name.clone(),
                access: e.access,
                hit_count: e.hit_count,
                last_op: e.last_op,
            })
            .collect();

        if clear {
            drop(recent); // Explicitly drop the MutexGuard
            self.recent.lock().unwrap().clear();
        }

        let open_handles = self.open_handles.load(Ordering::Relaxed);

        Snapshot {
            ops,
            recent_paths,
            open_handles,
            uptime: self.created_at.elapsed(),
            source: self.source.lock().unwrap().clone(),
            mountpoint: self.mountpoint.lock().unwrap().clone(),
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn new_stats_collector_starts_empty() {
        let stats = StatsCollector::new();
        let snap = stats.snapshot(false);
        for (total, ticks) in snap.ops.values() {
            assert_eq!(*total, 0);
            assert_eq!(*ticks, 0);
        }
        assert!(snap.recent_paths.is_empty());
        assert_eq!(snap.open_handles, 0);
    }

    #[test]
    fn record_single_op_reflected_in_snapshot() {
        let stats = StatsCollector::new();
        let p = Path::new("/tmp/test.txt");
        stats.record_op(OpType::Read, p, 100, AccessKind::Allowed);

        let snap = stats.snapshot(false);
        assert_eq!(snap.ops[&OpType::Read].0, 1); // total
        assert_eq!(snap.ops[&OpType::Read].1, 1); // tick
    }

    #[test]
    fn snapshot_resets_tick_counters() {
        let stats = StatsCollector::new();
        let p = Path::new("/tmp/test.txt");
        stats.record_op(OpType::Read, p, 100, AccessKind::Allowed);

        let snap1 = stats.snapshot(false);
        assert_eq!(snap1.ops[&OpType::Read].1, 1);

        // After snapshot, tick counter should be zero
        let snap2 = stats.snapshot(false);
        assert_eq!(snap2.ops[&OpType::Read].1, 0);
        // Total should persist
        assert_eq!(snap2.ops[&OpType::Read].0, 1);
    }

    #[test]
    fn multiple_ops_accumulate_totals() {
        let stats = StatsCollector::new();
        let p = Path::new("/tmp/test.txt");

        for _ in 0..10 {
            stats.record_op(OpType::Lookup, p, 100, AccessKind::Allowed);
        }
        for _ in 0..5 {
            stats.record_op(OpType::Read, p, 100, AccessKind::Allowed);
        }

        let snap = stats.snapshot(false);
        assert_eq!(snap.ops[&OpType::Lookup].0, 10);
        assert_eq!(snap.ops[&OpType::Read].0, 5);
    }

    #[test]
    fn recent_paths_maintains_unique_entries() {
        let stats = StatsCollector::new();
        let p1 = Path::new("/tmp/file1.txt");
        let p2 = Path::new("/tmp/file2.txt");

        stats.record_op(OpType::Read, p1, 100, AccessKind::Allowed);
        stats.record_op(OpType::Read, p2, 101, AccessKind::Allowed);
        stats.record_op(OpType::Read, p1, 100, AccessKind::Allowed); // duplicate

        let snap = stats.snapshot(false);
        assert_eq!(snap.recent_paths.len(), 2);

        // Most recent should be file1 (last accessed)
        assert!(snap.recent_paths[0].path.ends_with("file1.txt"));
        // Hit count should be 2 for file1
        assert_eq!(snap.recent_paths[0].hit_count, 2);
        // Hit count should be 1 for file2
        assert!(snap.recent_paths[1].path.ends_with("file2.txt"));
        assert_eq!(snap.recent_paths[1].hit_count, 1);
    }

    #[test]
    fn recent_paths_max_10_oldest_dropped() {
        let stats = StatsCollector::new();

        for i in 0..12 {
            let p_str = format!("/tmp/file{i}.txt");
            let p = Path::new(&p_str);
            stats.record_op(OpType::Read, p, 100, AccessKind::Allowed);
        }

        let snap = stats.snapshot(false);
        assert_eq!(snap.recent_paths.len(), 10);

        // The first two files (file0, file1) should be gone
        assert!(
            !snap
                .recent_paths
                .iter()
                .any(|ps| ps.path.ends_with("file0.txt"))
        );
        assert!(
            !snap
                .recent_paths
                .iter()
                .any(|ps| ps.path.ends_with("file1.txt"))
        );
        // The last one (file11) should be present (newest)
        assert!(snap.recent_paths[0].path.ends_with("file11.txt"));
    }

    #[test]
    fn recent_paths_updates_hit_count_on_re_access() {
        let stats = StatsCollector::new();
        let p = Path::new("/tmp/repeated.txt");

        for _ in 0..5 {
            stats.record_op(OpType::Read, p, 100, AccessKind::Allowed);
        }

        let snap = stats.snapshot(false);
        assert_eq!(snap.recent_paths.len(), 1);
        assert_eq!(snap.recent_paths[0].hit_count, 5);
    }

    #[test]
    fn recent_paths_tracks_process_info() {
        let stats = StatsCollector::new();
        let p = Path::new("/tmp/proc_test.txt");
        let our_pid = std::process::id();

        stats.record_op(OpType::Read, p, our_pid, AccessKind::Allowed);

        let snap = stats.snapshot(false);
        assert_eq!(snap.recent_paths[0].pid, our_pid);
        assert!(!snap.recent_paths[0].process_name.is_empty());
    }

    #[test]
    fn recent_paths_records_access_kind() {
        let stats = StatsCollector::new();
        let p = Path::new("/tmp/secret.txt");

        stats.record_op(OpType::Read, p, 100, AccessKind::Denied);

        let snap = stats.snapshot(false);
        assert_eq!(snap.recent_paths[0].access, AccessKind::Denied);
    }

    #[test]
    fn open_handles_inc_dec() {
        let stats = StatsCollector::new();

        stats.record_handle_open();
        stats.record_handle_open();
        stats.record_handle_open();
        let snap1 = stats.snapshot(false);
        assert_eq!(snap1.open_handles, 3);

        stats.record_handle_close();
        let snap2 = stats.snapshot(false);
        assert_eq!(snap2.open_handles, 2);
    }

    #[test]
    fn uptime_increases() {
        let stats = StatsCollector::new();
        let snap1 = stats.snapshot(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let snap2 = stats.snapshot(false);
        assert!(snap2.uptime > snap1.uptime);
    }

    #[test]
    fn source_and_mountpoint_persist() {
        let stats = StatsCollector::new();
        stats.set_source(PathBuf::from("/src"));
        stats.set_mountpoint(PathBuf::from("/mnt"));

        let snap = stats.snapshot(false);
        assert_eq!(snap.source, PathBuf::from("/src"));
        assert_eq!(snap.mountpoint, PathBuf::from("/mnt"));
    }

    #[test]
    fn denied_op_tracked_separately() {
        let stats = StatsCollector::new();
        let p = Path::new("/tmp/hidden.txt");

        stats.record_op(OpType::Denied, p, 100, AccessKind::Denied);

        let snap = stats.snapshot(false);
        assert!(snap.ops[&OpType::Denied].0 >= 1);
        assert_eq!(snap.recent_paths[0].access, AccessKind::Denied);
    }
}
