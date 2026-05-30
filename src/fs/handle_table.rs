//! Open-file handle table — maps FUSE file handles to real `std::fs::File` objects.

use fuser::FileHandle;
use std::collections::HashMap;

/// Manages a mapping from FUSE file handles to real filesystem file
/// descriptors.
pub struct HandleTable {
    next_fh: u64,
    handles: HashMap<u64, std::fs::File>,
}

impl HandleTable {
    /// Create an empty handle table.
    pub fn new() -> Self {
        Self {
            next_fh: 1,
            handles: HashMap::new(),
        }
    }

    /// Insert a file and return a new unique handle.
    pub fn insert(&mut self, file: std::fs::File) -> FileHandle {
        let fh = self.next_fh;
        self.next_fh += 1;
        self.handles.insert(fh, file);
        FileHandle(fh)
    }

    /// Look up a file by handle.
    pub fn get(&self, fh: FileHandle) -> Option<&std::fs::File> {
        self.handles.get(&fh.0)
    }

    /// Remove a file handle (closes the file on drop).
    pub fn remove(&mut self, fh: FileHandle) {
        self.handles.remove(&fh.0);
    }
}

impl Default for HandleTable {
    fn default() -> Self {
        Self::new()
    }
}
