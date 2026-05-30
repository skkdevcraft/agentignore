//! Policy engine — combines `.agentignore`-based hiding with cascading
//! `.agentallow`-based process bypasses.

use crate::fs::allow_list::CascadingAllowList;
use fuser::Request;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;
use tracing::{debug, warn};

/// The policy engine.
///
/// Owns:
/// - A root-level `Gitignore` (from the top-level `.agentignore`)
/// - Per-directory matchers (from subdirectory `.agentignore` files)
/// - A [`CascadingAllowList`] for process-based bypass rules
///
/// Hot-reloads when any config file's mtime changes.
pub struct Policy {
    root: PathBuf,
    root_matcher: ignore::gitignore::Gitignore,
    dir_matchers: HashMap<PathBuf, Option<ignore::gitignore::Gitignore>>,
    allow_list: CascadingAllowList,
    last_loaded: SystemTime,
    config_mtimes: HashMap<PathBuf, Option<SystemTime>>,
    /// Memoized mapping from queried path → directory key in `dir_matchers`
    /// that has the closest matcher.  An empty key means the root matcher is
    /// used.  Cleared on config reload via `std::mem::take`.
    matcher_cache: RwLock<HashMap<PathBuf, PathBuf>>,
}

impl Policy {
    /// Load (or reload) all policy config from `root`.
    pub fn load(root: &Path) -> Self {
        let agentignore = root.join(".agentignore");
        let agentallow = root.join(".agentallow");

        let mut config_mtimes = HashMap::new();
        config_mtimes.insert(agentignore.clone(), Self::get_mtime(&agentignore));
        config_mtimes.insert(agentallow.clone(), Self::get_mtime(&agentallow));

        // Root-level matcher
        let root_matcher = Self::build_matcher(root, &[root.to_path_buf()]);
        let allow_list = CascadingAllowList::load(root);

        // Per-directory matchers
        let mut dir_matchers = HashMap::new();
        Self::scan_dir_matchers(root, root, &mut dir_matchers, &[]);

        // Track mtimes for subdirectory config files
        for dir in dir_matchers.keys() {
            let cfg_path = dir.join(".agentignore");
            config_mtimes
                .entry(cfg_path.clone())
                .or_insert_with(|| Self::get_mtime(&cfg_path));
            let allow_path = dir.join(".agentallow");
            config_mtimes
                .entry(allow_path.clone())
                .or_insert_with(|| Self::get_mtime(&allow_path));
        }

        Self {
            root: root.to_path_buf(),
            root_matcher,
            dir_matchers,
            allow_list,
            last_loaded: SystemTime::now(),
            config_mtimes,
            matcher_cache: RwLock::new(HashMap::new()),
        }
    }

    fn build_matcher(root: &Path, config_dirs: &[PathBuf]) -> ignore::gitignore::Gitignore {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
        for dir in config_dirs {
            let agentignore = dir.join(".agentignore");
            if agentignore.exists()
                && let Some(err) = builder.add(&agentignore)
            {
                warn!("Error loading .agentignore from {:?}: {}", dir, err);
            }
        }
        builder.build().unwrap_or_else(|_| {
            ignore::gitignore::GitignoreBuilder::new(root)
                .build()
                .unwrap()
        })
    }

    fn scan_dir_matchers(
        root: &Path,
        current: &Path,
        matchers: &mut HashMap<PathBuf, Option<ignore::gitignore::Gitignore>>,
        ancestor_config_dirs: &[PathBuf],
    ) {
        if let Ok(entries) = std::fs::read_dir(current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let agentignore = path.join(".agentignore");
                    let mut current_config_dirs = ancestor_config_dirs.to_vec();
                    if agentignore.exists() {
                        current_config_dirs.push(path.clone());
                        let mut all_configs = vec![root.to_path_buf()];
                        all_configs.extend(current_config_dirs.iter().cloned());
                        let matcher = Self::build_matcher(root, &all_configs);
                        matchers.insert(path.clone(), Some(matcher));
                    } else {
                        matchers.insert(path.clone(), None);
                    }
                    Self::scan_dir_matchers(root, &path, matchers, &current_config_dirs);
                }
            }
        }
    }

    fn get_mtime(path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path).ok()?.modified().ok()
    }

    /// Return the most specific matcher for `path` (walks up the tree).
    ///
    /// Results are memoized in `matcher_cache` so that repeated lookups for
    /// the same path (common during `readdir`, `lookup` bursts) skip the
    /// directory walk entirely.
    fn get_matcher_for_path(&self, path: &Path) -> &ignore::gitignore::Gitignore {
        // ── cache hit ──────────────────────────────────────────────────────
        {
            let cache = self
                .matcher_cache
                .read()
                .expect("matcher_cache RwLock poisoned — fatal process state");
            if let Some(cached_dir) = cache.get(path) {
                if cached_dir.as_os_str().is_empty() {
                    return &self.root_matcher;
                }
                if let Some(Some(matcher)) = self.dir_matchers.get(cached_dir) {
                    return matcher;
                }
                // cached_dir exists but matcher vanished — fall through
            }
        }

        // ── cache miss — walk up the tree ──────────────────────────────────
        let mut current = path.to_path_buf();
        while current.starts_with(&self.root) {
            if let Some(Some(_)) = self.dir_matchers.get(&current) {
                self.matcher_cache
                    .write()
                    .expect("matcher_cache RwLock poisoned — fatal process state")
                    .insert(path.to_path_buf(), current.clone());
                // This unwrap is safe because we just checked for Some(Some(_)).
                return self.dir_matchers.get(&current).unwrap().as_ref().unwrap();
            }
            if current == self.root {
                break;
            }
            current = current.parent().unwrap_or(&self.root).to_path_buf();
        }

        // ── root fallback ──────────────────────────────────────────────────
        self.matcher_cache
            .write()
            .expect("matcher_cache RwLock poisoned — fatal process state")
            .insert(path.to_path_buf(), PathBuf::new());
        &self.root_matcher
    }

    /// Check if any config file has changed on disk, without mutating state.
    ///
    /// Returns `true` if any tracked config file's mtime has changed since the
    /// last full load or reload.  This is the read-only counterpart to
    /// `check_and_reload()` and avoids the write lock when nothing changed.
    pub fn has_config_changed(&self) -> bool {
        let agentignore = self.root.join(".agentignore");
        let agentallow = self.root.join(".agentallow");

        let current = Self::get_mtime(&agentignore);
        if current != self.config_mtimes.get(&agentignore).copied().flatten() {
            return true;
        }
        let current = Self::get_mtime(&agentallow);
        if current != self.config_mtimes.get(&agentallow).copied().flatten() {
            return true;
        }

        for dir in self.dir_matchers.keys() {
            let cfg = dir.join(".agentignore");
            let mtime = Self::get_mtime(&cfg);
            if mtime != self.config_mtimes.get(&cfg).copied().flatten() {
                return true;
            }
            let allow = dir.join(".agentallow");
            let mtime = Self::get_mtime(&allow);
            if mtime != self.config_mtimes.get(&allow).copied().flatten() {
                return true;
            }
        }

        false
    }

    /// Check if any config file has changed on disk and reload if so.
    ///
    /// Returns `true` if a reload occurred.
    pub fn check_and_reload(&mut self) -> bool {
        let agentignore = self.root.join(".agentignore");
        let agentallow = self.root.join(".agentallow");

        let current_agentignore_mtime = Self::get_mtime(&agentignore);
        let current_agentallow_mtime = Self::get_mtime(&agentallow);

        let mut needs_reload = current_agentignore_mtime
            != self.config_mtimes.get(&agentignore).copied().flatten()
            || current_agentallow_mtime != self.config_mtimes.get(&agentallow).copied().flatten();

        let mut dir_matchers = std::mem::take(&mut self.dir_matchers);
        for dir in dir_matchers.keys() {
            let agentignore = dir.join(".agentignore");
            let mtime = Self::get_mtime(&agentignore);
            if mtime != self.config_mtimes.get(&agentignore).copied().flatten() {
                needs_reload = true;
                self.config_mtimes.insert(agentignore, mtime);
            }

            let agentallow = dir.join(".agentallow");
            let allow_mtime = Self::get_mtime(&agentallow);
            if allow_mtime != self.config_mtimes.get(&agentallow).copied().flatten() {
                needs_reload = true;
                self.config_mtimes.insert(agentallow, allow_mtime);
            }
        }

        if needs_reload {
            debug!("Config files changed, reloading policy");

            self.root_matcher = Self::build_matcher(&self.root, &[self.root.to_path_buf()]);

            dir_matchers.clear();
            Self::scan_dir_matchers(&self.root, &self.root, &mut dir_matchers, &[]);

            self.allow_list = CascadingAllowList::load(&self.root);
            self.last_loaded = SystemTime::now();
            self.matcher_cache
                .write()
                .expect("matcher_cache RwLock poisoned — fatal process state")
                .clear();

            self.config_mtimes
                .insert(agentignore.clone(), current_agentignore_mtime);
            self.config_mtimes
                .insert(agentallow.clone(), current_agentallow_mtime);

            self.dir_matchers = dir_matchers;

            debug!("Policy reloaded successfully");
        } else {
            self.dir_matchers = dir_matchers;
        }

        needs_reload
    }

    /// Whether `real_path` is hidden according to `.agentignore` rules.
    ///
    /// The `.agentignore` and `.agentallow` files themselves are always hidden.
    pub fn is_hidden(&self, real_path: &Path) -> bool {
        if let Some(name) = real_path.file_name()
            && (name == ".agentignore" || name == ".agentallow")
        {
            return true;
        }

        let is_dir = real_path.is_dir();
        let matcher = self.get_matcher_for_path(real_path);
        matches!(
            matcher.matched_path_or_any_parents(real_path, is_dir),
            ignore::Match::Ignore(_)
        )
    }

    /// Check if a request context is allowed to bypass hiding rules for a path.
    pub fn is_request_allowed(&self, path: &Path, req: &Request) -> bool {
        self.allow_list.is_allowed(path, req.pid())
    }

    /// Check if a specific PID is allowed to bypass hiding for a path.
    pub fn is_allowed_raw(&self, path: &Path, pid: u32) -> bool {
        self.allow_list.is_allowed(path, pid)
    }
}
