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
    let (path, _resolved, ino) = fs
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
    let (path, _resolved, _ino) = fs
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
    std::os::unix::fs::symlink(root.join("target"), root.join("link")).unwrap();

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
    let (_, _, ino1) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("same.txt"), None)
        .unwrap();
    let (_, _, ino2) = fs
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
    let (_, _, ino1) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("one.txt"), None)
        .unwrap();
    let (_, _, ino2) = fs
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

    let (a_path, _, a_ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("a"), None)
        .unwrap();
    assert!(a_path.ends_with("a"));

    let (b_path, _, b_ino) = fs
        .lookup_child(INodeNo(a_ino), OsStr::new("b"), None)
        .unwrap();
    assert!(b_path.ends_with("b"));

    let (c_path, _, _) = fs
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
    std::os::unix::fs::symlink(root.join("realfile.txt"), root.join("link.txt")).unwrap();

    let fs = AgentFS::new(root.clone());
    let (path, resolved, _ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("link.txt"), None)
        .unwrap();
    // After the fix, lookup_child returns the access path (the symlink),
    // not the canonicalized target. The resolved target is returned separately.
    assert!(path.ends_with("link.txt"));
    // But the resolved target should point to realfile.txt
    assert_eq!(resolved, Some(root.join("realfile.txt")));
}

#[test]
fn agentfs_lookup_child_symlink_to_hidden_target_returns_none() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, ".env\n");
    common::touch(&root.join(".env"));
    std::os::unix::fs::symlink(root.join(".env"), root.join("link_to_env")).unwrap();

    let fs = AgentFS::new(root.clone());
    assert!(
        fs.lookup_child(INodeNo::ROOT, OsStr::new("link_to_env"), None)
            .is_none()
    );
}

#[test]
fn agentfs_lookup_child_dangling_symlink_returns_some() {
    // A dangling symlink still exists as an entry — it should be visible.
    let (_dir, root) = common::test_dir();
    std::os::unix::fs::symlink(root.join("nonexistent_target"), root.join("dangling")).unwrap();

    let fs = AgentFS::new(root.clone());
    let (path, resolved, _ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("dangling"), None)
        .unwrap();
    assert!(path.ends_with("dangling"));
    // Resolved is None because canonicalize fails (target doesn't exist)
    assert!(resolved.is_none());
}

#[test]
fn agentfs_symlink_and_target_get_distinct_inodes() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("real_file"));
    std::os::unix::fs::symlink(root.join("real_file"), root.join("the_link")).unwrap();

    let fs = AgentFS::new(root.clone());
    let (link_path, _, link_ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("the_link"), None)
        .unwrap();
    let (target_path, _, target_ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("real_file"), None)
        .unwrap();

    // Inodes must be different — symlink != target
    assert_ne!(link_ino, target_ino);
    // The link access path is the symlink itself
    assert!(link_path.ends_with("the_link"));
    // The target access path is the real file
    assert!(target_path.ends_with("real_file"));
}

#[test]
fn agentfs_two_symlinks_to_same_target_get_distinct_inodes() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("target.txt"));
    std::os::unix::fs::symlink(root.join("target.txt"), root.join("link_a")).unwrap();
    std::os::unix::fs::symlink(root.join("target.txt"), root.join("link_b")).unwrap();

    let fs = AgentFS::new(root.clone());
    let (_, _, ino_a) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("link_a"), None)
        .unwrap();
    let (_, _, ino_b) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("link_b"), None)
        .unwrap();

    assert_ne!(
        ino_a, ino_b,
        "two symlinks to same target must have distinct inodes"
    );
}

#[test]
fn agentfs_unlink_symlink_removes_symlink_not_target() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("precious.txt"));
    std::os::unix::fs::symlink(root.join("precious.txt"), root.join("link_to_precious")).unwrap();

    // Verify both exist
    assert!(root.join("precious.txt").exists());
    assert!(root.join("link_to_precious").exists());

    // Use lookup_child to get the access path (the symlink node), then remove it
    let fs = AgentFS::new(root.clone());
    let (access_path, _, _ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("link_to_precious"), None)
        .unwrap();

    // Remove the symlink via its access path (not the resolved target)
    std::fs::remove_file(&access_path).unwrap();

    // The target must survive
    assert!(
        root.join("precious.txt").exists(),
        "target file was removed!"
    );
    // The symlink itself is gone
    assert!(
        !root.join("link_to_precious").exists(),
        "symlink was not removed"
    );
}

#[test]
fn agentfs_stat_on_symlink_returns_correct_size() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("target_large"));
    // Write some content so target has a non-zero size
    std::fs::write(root.join("target_large"), b"some content here").unwrap();
    let link_path = root.join("mylink");
    let target_path = root.join("target_large");
    // Create a symlink. The size should be the length of the target path string.
    std::os::unix::fs::symlink(&target_path, &link_path).unwrap();

    // The symlink's size on disk is the byte length of the target path
    let symlink_target_content = std::fs::read_link(&link_path).unwrap();
    let expected_symlink_size = symlink_target_content.as_os_str().len() as u64;

    let fs = AgentFS::new(root.clone());
    let attr = fs.stat(INodeNo(2), &link_path).unwrap();

    assert_eq!(attr.kind, FileType::Symlink);
    // The symlink's size is the length of the target path string, not the target file content size
    assert_eq!(
        attr.size, expected_symlink_size,
        "symlink size must be the length of the target path string"
    );
    // Verify the symlink size differs from the target file's content size
    let target_meta = std::fs::metadata(&target_path).unwrap();
    assert_ne!(
        attr.size,
        target_meta.len(),
        "symlink stat size must differ from target file size"
    );
}

#[test]
fn agentfs_resolve_child_hidden_check_denies_symlink_to_hidden() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, ".secret\n");
    common::touch(&root.join(".secret"));
    std::os::unix::fs::symlink(root.join(".secret"), root.join("sym_to_secret")).unwrap();

    let fs = AgentFS::new(root.clone());
    assert!(
        fs.lookup_child(INodeNo::ROOT, OsStr::new("sym_to_secret"), None)
            .is_none(),
        "symlink to hidden target should be rejected"
    );
}

#[test]
fn agentfs_resolve_child_accepts_symlink_to_visible_target() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, ".secret\n");
    common::touch(&root.join("visible.txt"));
    common::touch(&root.join(".secret"));
    std::os::unix::fs::symlink(root.join("visible.txt"), root.join("sym_to_visible")).unwrap();

    let fs = AgentFS::new(root.clone());
    let (path, resolved, _ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("sym_to_visible"), None)
        .unwrap();
    assert!(path.ends_with("sym_to_visible"));
    assert_eq!(resolved, Some(root.join("visible.txt")));
}
