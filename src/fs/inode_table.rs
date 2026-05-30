//! Inode table — maps inode numbers to real filesystem paths and vice versa.

use fuser::INodeNo;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A bidirectional mapping between inode numbers and real filesystem paths.
///
/// The root directory always gets inode `INodeNo::ROOT` (1).  New inodes are
/// assigned from a monotonically increasing counter starting at 2.
pub struct InodeTable {
    next_ino: u64,
    root: PathBuf,
    ino_to_path: HashMap<u64, PathBuf>,
    path_to_ino: HashMap<PathBuf, u64>,
}

impl InodeTable {
    /// Create a new table rooted at `root`.
    ///
    /// The root path is registered with inode 1 (`INodeNo::ROOT`).
    pub fn new(root: &Path) -> Self {
        let mut t = Self {
            next_ino: 2,
            root: root.to_path_buf(),
            ino_to_path: HashMap::new(),
            path_to_ino: HashMap::new(),
        };
        t.ino_to_path.insert(INodeNo::ROOT.0, root.to_path_buf());
        t.path_to_ino.insert(root.to_path_buf(), INodeNo::ROOT.0);
        t
    }

    /// Return the existing inode for `path`, or allocate a new one.
    pub fn get_or_insert(&mut self, path: &Path) -> u64 {
        if let Some(&ino) = self.path_to_ino.get(path) {
            return ino;
        }
        let ino = self.next_ino;
        self.next_ino += 1;
        self.ino_to_path.insert(ino, path.to_path_buf());
        self.path_to_ino.insert(path.to_path_buf(), ino);
        ino
    }

    /// Look up the real path for a given inode.
    pub fn path(&self, ino: u64) -> Option<&PathBuf> {
        self.ino_to_path.get(&ino)
    }

    /// Return the inode for the parent of `real_dir`.
    ///
    /// Falls back to root inode if `real_dir` has no parent or the parent
    /// is not in the table yet.
    pub fn parent_ino(&self, real_dir: &Path) -> u64 {
        real_dir
            .parent()
            .and_then(|p| self.path_to_ino.get(p))
            .copied()
            .unwrap_or(INodeNo::ROOT.0)
    }

    /// Remove all entries whose path is under `prefix` (but keep the root).
    pub fn evict_prefix(&mut self, prefix: &Path) {
        let victims: Vec<u64> = self
            .ino_to_path
            .iter()
            .filter(|(_, p)| p.starts_with(prefix) && **p != self.root)
            .map(|(&ino, _)| ino)
            .collect();
        for ino in victims {
            if let Some(p) = self.ino_to_path.remove(&ino) {
                self.path_to_ino.remove(&p);
            }
        }
    }
}
