//! `PolicyFreshnessGuard` — read-check / write-reload split for policy freshness.
//!
//! Most FUSE operations call `ensure_policy_fresh()` at their entry point.
//! Before this module, that method unconditionally acquired the policy
//! write-lock, serialising all concurrent FUSE operations.
//!
//! This guard splits the check into two phases:
//!
//! 1. **Fast path:** Compare a cached `last_check` timestamp (atomic u64)
//!    against wall-clock time.  If the last check was <1 second ago, return
//!    immediately — no locks at all.
//! 2. **Read path:** If enough time has passed, call `Policy::has_config_changed()`
//!    under the `RwLock::read`.  If nothing changed, update the timestamp and
//!    return.
//! 3. **Write path:** Only when `has_config_changed()` returns `true` do we
//!    acquire the `RwLock::write` and call `check_and_reload()`.

use crate::fs::inode_table::InodeTable;
use crate::fs::policy::Policy;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::RwLockReadGuard;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Encapsulates the read-check / write-reload protocol for policy freshness.
///
/// Owns the `RwLock<Policy>` and `RwLock<InodeTable>` that were previously
/// stored directly on `AgentIgnore`.  The guard provides read and write accessor
/// methods so callers can still interact with the policy and inode table
/// through the usual API.
pub struct PolicyFreshnessGuard {
    policy: RwLock<Policy>,
    inodes: RwLock<InodeTable>,
    root: PathBuf,
    last_check: AtomicU64,
    /// Minimum interval between config-mtime rechecks (in seconds).
    /// Defaults to 1.  Set to 0 to disable the fast-path entirely.
    check_interval_secs: AtomicU64,
}

impl PolicyFreshnessGuard {
    /// Create a new guard, loading the initial policy and inode table.
    pub fn new(root: &Path) -> Self {
        Self {
            policy: RwLock::new(Policy::load(root)),
            inodes: RwLock::new(InodeTable::new(root)),
            root: root.to_path_buf(),
            last_check: AtomicU64::new(0),
            check_interval_secs: AtomicU64::new(1),
        }
    }

    /// Create a new guard with a custom check interval.
    ///
    /// `interval_secs` controls the minimum time (in seconds) between
    /// config-mtime rechecks.  Set to 0 to always re-check on every call
    /// (useful in tests).
    pub fn with_check_interval(root: &Path, interval_secs: u64) -> Self {
        Self {
            policy: RwLock::new(Policy::load(root)),
            inodes: RwLock::new(InodeTable::new(root)),
            root: root.to_path_buf(),
            last_check: AtomicU64::new(0),
            check_interval_secs: AtomicU64::new(interval_secs),
        }
    }

    /// Set the check interval (in seconds).
    pub fn set_check_interval(&self, interval_secs: u64) {
        self.check_interval_secs
            .store(interval_secs, Ordering::Relaxed);
    }

    /// Ensure the policy is fresh, returning `true` if a reload occurred.
    ///
    /// The vast majority of calls return immediately on the fast path,
    /// making the per-op overhead a single `AtomicU64::load` plus a
    /// timestamp comparison.  When a reload happens, the inode table is
    /// also evicted so that hidden/unhidden files are reflected.
    pub fn ensure_fresh(&self) -> bool {
        let now_secs = now_unix_secs();
        let last = self.last_check.load(Ordering::Relaxed);

        // ── Fast path: skip entirely if checked recently ───────────────────
        let interval = self.check_interval_secs.load(Ordering::Relaxed);
        if interval > 0 && now_secs < last.wrapping_add(interval) {
            return false;
        }

        // ── Read path: check mtimes without mutating state ─────────────────
        {
            let policy = self
                .policy
                .read()
                .expect("policy RwLock poisoned — fatal process state");
            if !policy.has_config_changed() {
                self.last_check.store(now_secs, Ordering::Relaxed);
                return false;
            }
        }

        // ── Write path: something changed, reload ──────────────────────────
        {
            let mut policy = self
                .policy
                .write()
                .expect("policy RwLock poisoned — fatal process state");
            if policy.check_and_reload() {
                self.inodes
                    .write()
                    .expect("inodes RwLock poisoned — fatal process state")
                    .evict_prefix(&self.root);
                self.last_check.store(now_secs, Ordering::Relaxed);
                return true;
            }
        }

        self.last_check.store(now_secs, Ordering::Relaxed);
        false
    }

    // ── Accessors ──────────────────────────────────────────────────────────

    /// Acquire a read guard on the policy.
    #[inline]
    pub fn policy_read(&self) -> RwLockReadGuard<'_, Policy> {
        self.policy
            .read()
            .expect("policy RwLock poisoned — fatal process state")
    }

    /// Acquire a write guard on the policy.
    #[inline]
    pub fn policy_write(&self) -> std::sync::RwLockWriteGuard<'_, Policy> {
        self.policy
            .write()
            .expect("policy RwLock poisoned — fatal process state")
    }

    /// Acquire a read guard on the inode table.
    #[inline]
    pub fn inodes_read(&self) -> RwLockReadGuard<'_, InodeTable> {
        self.inodes
            .read()
            .expect("inodes RwLock poisoned — fatal process state")
    }

    /// Acquire a write guard on the inode table.
    #[inline]
    pub fn inodes_write(&self) -> std::sync::RwLockWriteGuard<'_, InodeTable> {
        self.inodes
            .write()
            .expect("inodes RwLock poisoned — fatal process state")
    }
}

/// Return the current Unix timestamp in whole seconds.
fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
