use agentignore::fs::InodeTable;

use fuser::INodeNo;

use std::path::PathBuf;

mod common;

#[test]
fn inode_table_new_has_root() {
    let (_dir, root) = common::test_dir();
    let t = InodeTable::new(&root);
    assert_eq!(t.path(INodeNo::ROOT.0), Some(&root));
}

#[test]
fn inode_table_path_returns_none_for_unknown() {
    let (_dir, root) = common::test_dir();
    let t = InodeTable::new(&root);
    assert!(t.path(9999).is_none());
}

#[test]
fn inode_table_get_or_insert_returns_same_ino_for_same_path() {
    let (_dir, root) = common::test_dir();
    let mut t = InodeTable::new(&root);
    let child = root.join("foo");
    let ino1 = t.get_or_insert(&child);
    let ino2 = t.get_or_insert(&child);
    assert_eq!(ino1, ino2);
}

#[test]
fn inode_table_get_or_insert_distinct_inos() {
    let (_dir, root) = common::test_dir();
    let mut t = InodeTable::new(&root);
    let a = t.get_or_insert(&root.join("a"));
    let b = t.get_or_insert(&root.join("b"));
    assert_ne!(a, b);
}

#[test]
fn inode_table_get_or_insert_starts_after_root() {
    let (_dir, root) = common::test_dir();
    let mut t = InodeTable::new(&root);
    let ino = t.get_or_insert(&root.join("x"));
    assert!(ino >= 2, "expected ino >= 2, got {ino}");
}

#[test]
fn inode_table_path_roundtrip() {
    let (_dir, root) = common::test_dir();
    let mut t = InodeTable::new(&root);
    let p = root.join("deep").join("nested").join("file.rs");
    let ino = t.get_or_insert(&p);
    assert_eq!(t.path(ino), Some(&p));
}

#[test]
fn inode_table_parent_ino_root() {
    let (_dir, root) = common::test_dir();
    let t = InodeTable::new(&root);
    assert_eq!(t.parent_ino(&root), INodeNo::ROOT.0);
}

#[test]
fn inode_table_parent_ino_nested() {
    let (_dir, root) = common::test_dir();
    let mut t = InodeTable::new(&root);
    let parent = root.join("parent");
    let child = parent.join("child");
    let parent_ino = t.get_or_insert(&parent);
    t.get_or_insert(&child);
    assert_eq!(t.parent_ino(&child), parent_ino);
}

#[test]
fn inode_table_parent_ino_falls_back_to_root() {
    let (_dir, root) = common::test_dir();
    let t = InodeTable::new(&root);
    assert_eq!(t.parent_ino(&PathBuf::from("/")), INodeNo::ROOT.0);
}

#[test]
fn inode_table_evict_prefix_removes_matching() {
    let (_dir, root) = common::test_dir();
    let mut t = InodeTable::new(&root);
    let a = root.join("a");
    let b = root.join("b");

    let a_ino = t.get_or_insert(&a);
    let _b_ino = t.get_or_insert(&b);

    // After eviction, re-inserting the same path gives a new inode number
    t.evict_prefix(&a);
    let a_ino_after = t.get_or_insert(&a);
    assert_ne!(a_ino, a_ino_after, "evicted path should get a new ino");
}

#[test]
fn inode_table_evict_prefix_preserves_root() {
    let (_dir, root) = common::test_dir();
    let mut t = InodeTable::new(&root);
    let root_ino = INodeNo::ROOT.0;
    t.get_or_insert(&root.join("x"));
    t.get_or_insert(&root.join("y"));

    t.evict_prefix(&root);

    // Root is preserved; other entries are gone (re-insert gets new ino)
    assert_eq!(t.path(root_ino), Some(&root));
}
