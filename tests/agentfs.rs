use agentignore::fs::AgentFS;

use fuser::{FileType, INodeNo};

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

mod common;

#[test]
fn agentfs_new_stores_root() {
    let (_dir, root) = common::test_dir();
    let fs = AgentFS::new(root.clone());
    assert_eq!(fs.root, root);
}

#[test]
fn agentfs_is_hidden_delegates_to_policy() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "secret.txt\n");
    common::touch(&root.join("secret.txt"));
    common::touch(&root.join("visible.txt"));

    let fs = AgentFS::new(root.clone());
    assert!(fs.is_hidden(&root.join("secret.txt")));
    assert!(!fs.is_hidden(&root.join("visible.txt")));
}

#[test]
fn agentfs_real_path_returns_none_for_unknown_ino() {
    let (_dir, root) = common::test_dir();
    let fs = AgentFS::new(root);
    assert!(fs.real_path(INodeNo(9999)).is_none());
}

#[test]
fn agentfs_real_path_returns_root_for_root_ino() {
    let (_dir, root) = common::test_dir();
    let fs = AgentFS::new(root.clone());
    assert_eq!(fs.real_path(INodeNo::ROOT), Some(root));
}

#[test]
fn agentfs_lookup_child_nonexistent() {
    let (_dir, root) = common::test_dir();
    let fs = AgentFS::new(root.clone());
    assert!(
        fs.lookup_child(INodeNo::ROOT, OsStr::new("nonexistent"), None)
            .is_none()
    );
}

#[test]
fn agentfs_lookup_child_finds_existing_file() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("hello.txt"));
    let fs = AgentFS::new(root.clone());
    let (path, ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("hello.txt"), None)
        .unwrap();
    assert!(path.ends_with("hello.txt"));
    assert!(ino >= 2);
}

#[test]
fn agentfs_lookup_child_hidden_file() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.secret\n");
    common::touch(&root.join("data.secret"));
    let fs = AgentFS::new(root.clone());
    assert!(
        fs.lookup_child(INodeNo::ROOT, OsStr::new("data.secret"), None)
            .is_none()
    );
}

#[test]
fn agentfs_lookup_child_path_escape() {
    let (_dir, root) = common::test_dir();
    let escape_target = if cfg!(target_os = "linux") {
        PathBuf::from("/etc/passwd")
    } else {
        PathBuf::from("/etc/hosts")
    };
    let link_path = root.join("escape");
    std::os::unix::fs::symlink(&escape_target, &link_path).unwrap();

    let fs = AgentFS::new(root.clone());
    assert!(
        fs.lookup_child(INodeNo::ROOT, OsStr::new("escape"), None)
            .is_none()
    );
}

#[test]
fn agentfs_lookup_child_finds_directory() {
    let (_dir, root) = common::test_dir();
    common::mkdirp(&root.join("mydir"));
    let fs = AgentFS::new(root.clone());
    let (path, _ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("mydir"), None)
        .unwrap();
    assert!(path.ends_with("mydir"));
    assert!(path.is_dir());
}

#[test]
fn agentfs_stat_returns_attributes_for_file() {
    let (_dir, root) = common::test_dir();
    let path = root.join("stat_test.txt");
    fs::write(&path, b"hello world").unwrap();

    let fs = AgentFS::new(root.clone());
    let attr = fs.stat(INodeNo::ROOT, &path).unwrap();
    assert_eq!(attr.size, 11);
    assert_eq!(attr.kind, FileType::RegularFile);
}

#[test]
fn agentfs_stat_returns_attributes_for_directory() {
    let (_dir, root) = common::test_dir();
    common::mkdirp(&root.join("somedir"));

    let fs = AgentFS::new(root.clone());
    let attr = fs.stat(INodeNo::ROOT, &root.join("somedir")).unwrap();
    assert_eq!(attr.kind, FileType::Directory);
}

#[test]
fn agentfs_stat_returns_none_for_nonexistent() {
    let (_dir, root) = common::test_dir();
    let fs = AgentFS::new(root);
    assert!(
        fs.stat(INodeNo::ROOT, &PathBuf::from("/nonexistent_path_12345"))
            .is_none()
    );
}

#[test]
fn agentfs_stat_for_symlink() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("target"));
    std::os::unix::fs::symlink(&root.join("target"), &root.join("link")).unwrap();

    let fs = AgentFS::new(root.clone());
    let attr = fs.stat(INodeNo::ROOT, &root.join("link")).unwrap();
    assert_eq!(attr.kind, FileType::Symlink);
}

#[test]
fn agentfs_stat_inode_number() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("foo.txt"));
    let fs = AgentFS::new(root.clone());
    let attr = fs.stat(INodeNo(42), &root.join("foo.txt")).unwrap();
    assert_eq!(attr.ino, INodeNo(42));
}

#[test]
fn agentfs_lookup_child_caches_inode() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("same.txt"));
    let fs = AgentFS::new(root.clone());
    let (_, ino1) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("same.txt"), None)
        .unwrap();
    let (_, ino2) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("same.txt"), None)
        .unwrap();
    assert_eq!(ino1, ino2);
}

#[test]
fn agentfs_lookup_child_different_inodes() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("one.txt"));
    common::touch(&root.join("two.txt"));
    let fs = AgentFS::new(root.clone());
    let (_, ino1) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("one.txt"), None)
        .unwrap();
    let (_, ino2) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("two.txt"), None)
        .unwrap();
    assert_ne!(ino1, ino2);
}

#[test]
fn agentfs_lookup_child_nested() {
    let (_dir, root) = common::test_dir();
    common::mkdirp(&root.join("a").join("b"));
    common::touch(&root.join("a").join("b").join("c.txt"));
    let fs = AgentFS::new(root.clone());

    let (a_path, a_ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("a"), None)
        .unwrap();
    assert!(a_path.ends_with("a"));

    let (b_path, b_ino) = fs
        .lookup_child(INodeNo(a_ino), OsStr::new("b"), None)
        .unwrap();
    assert!(b_path.ends_with("b"));

    let (c_path, _) = fs
        .lookup_child(INodeNo(b_ino), OsStr::new("c.txt"), None)
        .unwrap();
    assert!(c_path.ends_with("c.txt"));
}

#[test]
fn agentfs_lookup_child_hidden_directory() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "secret/\n");
    common::mkdirp(&root.join("secret"));
    let fs = AgentFS::new(root.clone());
    assert!(
        fs.lookup_child(INodeNo::ROOT, OsStr::new("secret"), None)
            .is_none()
    );
}

#[test]
fn agentfs_lookup_child_symlink_to_file_inside_root() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("realfile.txt"));
    std::os::unix::fs::symlink(&root.join("realfile.txt"), &root.join("link.txt")).unwrap();

    let fs = AgentFS::new(root.clone());
    let (path, _ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("link.txt"), None)
        .unwrap();
    // lookup_child canonicalizes, so the path resolves through the symlink
    assert!(path.ends_with("realfile.txt"));
}
