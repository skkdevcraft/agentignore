// Shared test helpers for agentfs integration tests.
// Each test binary compiles this file independently, so not every function is used
// in every binary. That is expected and harmless.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub fn test_dir() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

pub fn touch(path: &Path) {
    fs::write(path, b"data").unwrap();
}

pub fn mkdirp(path: &Path) {
    fs::create_dir_all(path).unwrap();
}

pub fn make_agentignore(root: &Path, content: &str) {
    fs::write(root.join(".agentignore"), content).unwrap();
}

pub fn make_agentallow(root: &Path, content: &str) {
    fs::write(root.join(".agentallow"), content).unwrap();
}

pub fn make_agentallow_in_dir(dir: &Path, content: &str) {
    fs::write(dir.join(".agentallow"), content).unwrap();
}
