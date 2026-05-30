//! Allow list — configures process-based bypasses via `.agentallow` files.
//!
//! # File format (`.agentallow`)
//!
//! - One entry per line.
//! - Empty lines and lines starting with `#` are ignored (comments).
//!
//! ## Process name / cmdline matching
//!
//! | Syntax | Behaviour |
//! |--------|-----------|
//! | `node` | Regex; matched against the process `comm` name OR its full
//!   cmdline.  The match also walks up the ancestor chain (parent, grandparent,
//!   etc.). |
//! | `=npm` | Exact match against `comm` OR cmdline; ancestors are also checked. |
//! | `=bash!` | Exact, no ancestor walk — matches only the exact process. |
//! | `node!` | Regex, no ancestor walk. |
//!
//! ## Binary path matching (entry starts with `/`)
//!
//! | Syntax | Behaviour |
//! |--------|-----------|
//! | `/usr/bin/node` | Literal path; matches process or any ancestor. |
//! | `/usr/bin/java!` | Literal path; matches only that exact process. |

use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tracing::{debug, warn};

// ── Pattern matching ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum ProcessPattern {
    /// Regex matched against comm name or cmdline
    Regex(Regex),
    /// Exact string matched against comm name or cmdline
    Exact(String),
}

impl ProcessPattern {
    fn matches(&self, comm: &str, cmdline: &str) -> bool {
        match self {
            ProcessPattern::Regex(re) => re.is_match(comm) || re.is_match(cmdline),
            ProcessPattern::Exact(s) => comm == s || cmdline == s,
        }
    }
}

/// Parse a process name config token (before any trailing `!` is removed).
/// `=foo`  → exact match for "foo"
/// `foo`   → regex /foo/
fn parse_process_pattern(token: &str) -> Option<ProcessPattern> {
    if let Some(exact) = token.strip_prefix('=') {
        Some(ProcessPattern::Exact(exact.to_string()))
    } else {
        match Regex::new(token) {
            Ok(re) => Some(ProcessPattern::Regex(re)),
            Err(e) => {
                warn!("Invalid regex in .agentallow {:?}: {}", token, e);
                None
            }
        }
    }
}

// ── Allow entries ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum AllowEntry {
    /// Match process/cmdline (regex or exact) or any ancestor
    ProcessName(ProcessPattern),
    /// Match process/cmdline (regex or exact), NO ancestor walk
    ProcessNameExact(ProcessPattern),
    /// Match binary path or any ancestor
    BinaryPath(PathBuf),
    /// Match exact binary path only (no ancestor walk)
    BinaryPathExact(PathBuf),
}

// ── Process info cache ──────────────────────────────────────────────────────

/// Per-process data read from `/proc/<pid>/`.
///
/// Kept for [`PROC_CACHE_TTL`] to balance freshness with reduced I/O.
#[derive(Debug, Clone)]
struct CachedProcess {
    comm: String,
    cmdline: String,
    exe: Option<PathBuf>,
    ppid: Option<u32>,
    loaded_at: Instant,
}

impl CachedProcess {
    fn is_fresh(&self) -> bool {
        self.loaded_at.elapsed() < PROC_CACHE_TTL
    }
}

/// How long a cached `/proc/<pid>/` entry stays valid.
///
/// PID reuse is unlikely within this window under normal loads; the small
/// staleness risk is worth the I/O savings during bursty FUSE traffic.
const PROC_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(2);

/// Local process-info cache shared across one `AllowList` lifetime.
///
/// Populated lazily and naturally cleared on config reload (which creates a
/// fresh `AllowList`).
#[derive(Debug, Clone, Default)]
struct ProcessCache {
    map: HashMap<u32, CachedProcess>,
}

impl ProcessCache {
    fn get_or_load(&mut self, pid: u32) -> CachedProcess {
        if let Some(cached) = self.map.get(&pid)
            && cached.is_fresh()
        {
            return cached.clone();
        }

        let proc_info = load_process(pid);
        self.map.insert(pid, proc_info.clone());
        proc_info
    }
}

fn load_process(pid: u32) -> CachedProcess {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());

    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|bytes| {
            bytes
                .split(|&b| b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok();

    let ppid = get_parent_pid_from_proc(pid);

    CachedProcess {
        comm,
        cmdline,
        exe,
        ppid,
        loaded_at: Instant::now(),
    }
}

/// Get the parent PID from `/proc/<pid>/stat` (uncached).
fn get_parent_pid_from_proc(pid: u32) -> Option<u32> {
    let stat_content = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat_content.rfind(')')?;
    let rest = &stat_content[after_comm + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    fields.get(1)?.parse().ok()
}

// ── AllowList (single file) ─────────────────────────────────────────────────

/// An allow list loaded from a single `.agentallow` file.
#[derive(Debug)]
pub struct AllowList {
    entries: Vec<AllowEntry>,
    /// Memoized process info to avoid redundant `/proc/<pid>/` reads.
    process_cache: Mutex<ProcessCache>,
}

impl AllowList {
    /// Load the `.agentallow` file from `root`.
    pub fn load(root: &Path) -> Self {
        let path = root.join(".agentallow");
        Self::load_from_file(&path)
    }

    /// Load from an explicit path.
    pub fn load_from_file(path: &Path) -> Self {
        let mut entries = Vec::new();

        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                let line = line.trim();
                // Skip comments and empty lines
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                // Peel off optional trailing '!' for exact-process-only matching.
                let (token, ancestor_walk) = if let Some(t) = line.strip_suffix('!') {
                    (t.trim(), false)
                } else {
                    (line, true)
                };

                if token.starts_with('/') {
                    // Binary path — always a literal comparison.
                    let pb = PathBuf::from(token);
                    if ancestor_walk {
                        debug!("Allow binary path or child: {} from {:?}", token, path);
                        entries.push(AllowEntry::BinaryPath(pb));
                    } else {
                        debug!("Allow exact binary path: {} from {:?}", token, path);
                        entries.push(AllowEntry::BinaryPathExact(pb));
                    }
                } else {
                    // Process name / cmdline — regex or exact.
                    if let Some(pattern) = parse_process_pattern(token) {
                        if ancestor_walk {
                            debug!(
                                "Allow process pattern or child: {:?} from {:?}",
                                token, path
                            );
                            entries.push(AllowEntry::ProcessName(pattern));
                        } else {
                            debug!("Allow exact process pattern: {:?} from {:?}", token, path);
                            entries.push(AllowEntry::ProcessNameExact(pattern));
                        }
                    }
                }
            }
        }

        if !entries.is_empty() {
            debug!("Loaded {} allow entries from {:?}", entries.len(), path);
        }

        Self {
            entries,
            process_cache: Mutex::new(ProcessCache::default()),
        }
    }

    /// Whether this list has no entries at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Check if a process (by PID) is allowed according to this list.
    pub fn is_allowed(&self, pid: u32) -> bool {
        if self.entries.is_empty() {
            return false;
        }

        for entry in &self.entries {
            match entry {
                AllowEntry::ProcessName(pattern) => {
                    if self.is_process_or_child_by_name(pid, pattern, None) {
                        debug!("Allow PID {} as process or child matching pattern", pid);
                        return true;
                    }
                }
                AllowEntry::ProcessNameExact(pattern) => {
                    if self.is_process_or_child_by_name(pid, pattern, Some(0)) {
                        debug!("Allow PID {} by exact-process pattern match", pid);
                        return true;
                    }
                }
                AllowEntry::BinaryPath(path) => {
                    if self.is_process_or_child_by_path(pid, path) {
                        debug!("Allow PID {} as process or child of {:?}", pid, path);
                        return true;
                    }
                }
                AllowEntry::BinaryPathExact(path) => {
                    let info = self.get_process_info(pid);
                    if info.exe.as_deref() == Some(path) {
                        debug!("Allow PID {} by exact exe {:?}", pid, path);
                        return true;
                    }
                }
            }
        }

        false
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Look up process info, using the memoization cache.
    fn get_process_info(&self, pid: u32) -> CachedProcess {
        self.process_cache
            .lock()
            .expect("process_cache Mutex poisoned — fatal process state")
            .get_or_load(pid)
    }

    fn check_parent_chain<F>(&self, mut pid: u32, max_depth: Option<u32>, should_stop: F) -> bool
    where
        F: Fn(u32, &str, &str) -> bool,
    {
        let current_pid = std::process::id();
        let mut depth = 0u32;

        loop {
            let info = self.get_process_info(pid);
            debug!("pid={pid} name={:?} cmdline={:?}", info.comm, info.cmdline);

            if should_stop(pid, &info.comm, &info.cmdline) {
                return true;
            }

            // Enforce max depth: stop walking up the chain once we've
            // examined `max_depth` levels (inclusive of the starting pid).
            if let Some(max) = max_depth
                && depth >= max
            {
                break;
            }

            let ppid = info.ppid;

            match ppid {
                Some(0) | None => break,
                Some(parent) if parent == current_pid => break,
                Some(parent) => {
                    pid = parent;
                    depth += 1;
                }
            }
        }

        false
    }

    /// Check if the process or any ancestor (up to `max_depth`) matches the
    /// given pattern (against both comm name and full cmdline).
    ///
    /// `max_depth = None` walks the entire ancestor chain; `Some(0)` checks
    /// only the process identified by `pid` without walking ancestors.
    fn is_process_or_child_by_name(
        &self,
        pid: u32,
        pattern: &ProcessPattern,
        max_depth: Option<u32>,
    ) -> bool {
        debug!(
            "is_process_or_child_by_name pid={} pattern={:?} max_depth={:?}",
            pid, pattern, max_depth
        );

        self.check_parent_chain(pid, max_depth, |_pid, pname, cmdline| {
            pattern.matches(pname, cmdline)
        })
    }

    /// Check if the process or any of its ancestors has the given binary path
    fn is_process_or_child_by_path(&self, mut pid: u32, path: &Path) -> bool {
        loop {
            let info = self.get_process_info(pid);
            if info.exe.as_deref() == Some(path) {
                return true;
            }

            match info.ppid {
                Some(0) | None => return false,
                Some(parent) => {
                    pid = parent;
                }
            }
        }
    }
}

// ── CascadingAllowList ──────────────────────────────────────────────────────

/// A hierarchical allow list that scans `.agentallow` files in subdirectories
/// and combines them with the root-level one.
#[derive(Debug)]
pub struct CascadingAllowList {
    root: PathBuf,
    root_allow_list: AllowList,
    dir_allow_lists: HashMap<PathBuf, Option<AllowList>>,
}

impl CascadingAllowList {
    /// Load the root `.agentallow` and scan subdirectories for additional ones.
    pub fn load(root: &Path) -> Self {
        let root_allow_list = AllowList::load(root);

        let mut dir_allow_lists = HashMap::new();
        Self::scan_dir_allow_lists(root, &mut dir_allow_lists);

        Self {
            root: root.to_path_buf(),
            root_allow_list,
            dir_allow_lists,
        }
    }

    fn scan_dir_allow_lists(current: &Path, allow_lists: &mut HashMap<PathBuf, Option<AllowList>>) {
        if let Ok(entries) = std::fs::read_dir(current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let agentallow = path.join(".agentallow");
                    if agentallow.exists() {
                        let allow_list = AllowList::load_from_file(&agentallow);
                        allow_lists.insert(path.clone(), Some(allow_list));
                    } else {
                        allow_lists.insert(path.clone(), None);
                    }
                    Self::scan_dir_allow_lists(&path, allow_lists);
                }
            }
        }
    }

    /// Walk up the directory tree from `path` collecting every allow list
    /// found along the way (root first, then narrower scopes).
    fn get_combined_allow_lists_for_path(&self, path: &Path) -> Vec<&AllowList> {
        let mut allow_lists = Vec::new();
        allow_lists.push(&self.root_allow_list);

        let mut current = path.to_path_buf();
        while current.starts_with(&self.root) && current != self.root() {
            if let Some(Some(allow_list)) = self.dir_allow_lists.get(&current) {
                allow_lists.push(allow_list);
            }
            if let Some(parent) = current.parent() {
                current = parent.to_path_buf();
            } else {
                break;
            }
        }

        allow_lists
    }

    fn root(&self) -> &Path {
        &self.root
    }

    /// Check if a process is allowed for a specific path by consulting all
    /// applicable allow lists (root-level and any subdirectory lists).
    pub fn is_allowed(&self, path: &Path, pid: u32) -> bool {
        let allow_lists = self.get_combined_allow_lists_for_path(path);
        for allow_list in &allow_lists {
            if allow_list.is_allowed(pid) {
                return true;
            }
        }
        false
    }

    /// Whether any allow list (root or subdirectory) has entries.
    pub fn has_any_entries(&self) -> bool {
        if !self.root_allow_list.is_empty() {
            return true;
        }
        for allow_list_opt in self.dir_allow_lists.values() {
            if let Some(allow_list) = allow_list_opt
                && !allow_list.is_empty()
            {
                return true;
            }
        }
        false
    }
}
