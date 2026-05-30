use agentignore::fs::{AgentFS, Policy};

use fuser::INodeNo;

use std::ffi::OsStr;

mod common;

#[test]
fn agentfs_policy_loaded_at_construction() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "old.txt\n");
    common::touch(&root.join("old.txt"));
    common::touch(&root.join("new.txt"));

    let fs = AgentFS::new(root.clone());
    fs.set_check_interval(0);

    assert!(fs.is_hidden(&root.join("old.txt")));
    assert!(!fs.is_hidden(&root.join("new.txt")));

    // Wait to ensure mtime changes on filesystems with 1-second resolution
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Update .agentignore on disk; policy IS hot-reloaded via is_hidden.
    common::make_agentignore(&root, "old.txt\nnew.txt\n");

    assert!(fs.is_hidden(&root.join("old.txt")));
    assert!(fs.is_hidden(&root.join("new.txt")));
}

#[test]
fn policy_hot_reloads_when_agentignore_changes() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "old.txt\n");
    common::touch(&root.join("old.txt"));
    common::touch(&root.join("new.txt"));

    let fs = AgentFS::new(root.clone());
    fs.set_check_interval(0);

    // Initially, only old.txt is hidden
    assert!(fs.is_hidden(&root.join("old.txt")));
    assert!(!fs.is_hidden(&root.join("new.txt")));

    // Wait to ensure mtime changes on filesystems with 1-second resolution
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Update .agentignore to also hide new.txt
    common::make_agentignore(&root, "old.txt\nnew.txt\n");

    // Policy should hot-reload and now hide both files
    assert!(fs.is_hidden(&root.join("old.txt")));
    assert!(fs.is_hidden(&root.join("new.txt")));
}

#[test]
fn policy_hot_reloads_when_agentallow_changes() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "secret.txt\n");
    common::touch(&root.join("secret.txt"));

    let pid = std::process::id() as u32;

    // Initially allow this process by name
    let comm = std::fs::read_to_string("/proc/self/comm").unwrap();
    common::make_agentallow(&root, &format!("{}\n", comm.trim()));

    let fs = AgentFS::new(root.clone());

    // Verify policy allows this process
    {
        let policy = fs.policy_read();
        assert!(policy.is_allowed_raw(&root.join("secret.txt"), pid));
    }

    // Change .agentallow to not include this process
    std::thread::sleep(std::time::Duration::from_millis(10)); // Ensure mtime changes
    common::make_agentallow(&root, "nonexistent_process_name\n");

    // Trigger hot-reload via is_hidden before checking the reloaded policy
    fs.is_hidden(&root.join("secret.txt"));

    // Policy should hot-reload and no longer allow this process
    {
        let policy = fs.policy_read();
        assert!(!policy.is_allowed_raw(&root.join("secret.txt"), pid));
    }
}

#[test]
fn hot_reload_invalidates_inode_cache() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.secret\n");
    common::touch(&root.join("visible.rs"));
    common::touch(&root.join("test.txt"));

    let fs = AgentFS::new(root.clone());
    fs.set_check_interval(0);

    // Look up both files to cache their inodes (neither matches *.secret)
    let (_path1, ino1) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("visible.rs"), None)
        .unwrap();
    let (_path2, ino2) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("test.txt"), None)
        .unwrap();

    // Verify both files exist in cache
    assert!(fs.real_path(INodeNo(ino1)).is_some());
    assert!(fs.real_path(INodeNo(ino2)).is_some());

    // Wait to ensure mtime changes on filesystems with 1-second resolution
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Change .agentignore to hide nothing
    common::make_agentignore(&root, "# nothing hidden\n");

    // Force policy check (triggers hot-reload and inode cache eviction)
    fs.is_hidden(&root.join("visible.rs"));

    // Old inodes should be invalidated, but new lookups should work
    assert!(fs.real_path(INodeNo(ino1)).is_none());
    assert!(fs.real_path(INodeNo(ino2)).is_none());

    // Both files should now be visible and get new inodes
    let (_path3, new_ino1) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("visible.rs"), None)
        .unwrap();
    let (_path4, new_ino2) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("test.txt"), None)
        .unwrap();

    assert_ne!(ino1, new_ino1);
    assert_ne!(ino2, new_ino2);
    assert!(new_ino1 >= 2);
    assert!(new_ino2 >= 2);
}

#[test]
fn hot_reload_only_when_files_actually_changed() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "test.txt\n");
    common::touch(&root.join("test.txt"));

    let fs = AgentFS::new(root.clone());
    fs.set_check_interval(0);

    // Check initial state
    assert!(fs.is_hidden(&root.join("test.txt")));

    // Should not reload since mtime hasn't changed
    assert!(fs.is_hidden(&root.join("test.txt")));

    // Wait to ensure mtime changes on filesystems with 1-second resolution
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Now change the file
    common::make_agentignore(&root, "# nothing hidden\n");

    // Should reload and show the file
    assert!(!fs.is_hidden(&root.join("test.txt")));
}

// ── Cascading-agentignore hot-reload tests (still about hot-reload) ──

#[test]
fn cascading_agentignore_hot_reload_picks_up_new_subdir_config() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");

    common::mkdirp(&root.join("dynamic"));
    common::touch(&root.join("dynamic").join("new.gen"));
    common::touch(&root.join("dynamic").join("test.log"));

    let mut policy = Policy::load(&root);

    // Initially, only root rules apply
    assert!(policy.is_hidden(&root.join("dynamic").join("test.log")));
    assert!(!policy.is_hidden(&root.join("dynamic").join("new.gen")));

    // Create a new .agentignore in the subdirectory
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::fs::write(root.join("dynamic").join(".agentignore"), "*.gen\n").unwrap();

    // Hot reload should pick up the new config
    assert!(policy.check_and_reload());

    // Now both root and subdir rules apply
    assert!(policy.is_hidden(&root.join("dynamic").join("test.log")));
    assert!(policy.is_hidden(&root.join("dynamic").join("new.gen")));
}

#[test]
fn cascading_agentignore_hot_reload_detects_subdir_config_changes() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");

    common::mkdirp(&root.join("mutable"));
    std::fs::write(root.join("mutable").join(".agentignore"), "*.tmp\n").unwrap();

    common::touch(&root.join("mutable").join("file.tmp"));
    common::touch(&root.join("mutable").join("file.cache"));

    let mut policy = Policy::load(&root);

    // Initial state: root rules + subdir *.tmp
    assert!(policy.is_hidden(&root.join("mutable").join("file.tmp")));
    assert!(!policy.is_hidden(&root.join("mutable").join("file.cache")));

    // Modify the subdir .agentignore
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::fs::write(root.join("mutable").join(".agentignore"), "*.cache\n").unwrap();

    // Hot reload should detect the change
    assert!(policy.check_and_reload());

    // Now *.cache is hidden instead of *.tmp
    assert!(!policy.is_hidden(&root.join("mutable").join("file.tmp")));
    assert!(policy.is_hidden(&root.join("mutable").join("file.cache")));
}

#[test]
fn cascading_agentignore_subdir_config_removal_detected() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");

    common::mkdirp(&root.join("removable"));
    std::fs::write(root.join("removable").join(".agentignore"), "*.special\n").unwrap();

    common::touch(&root.join("removable").join("file.special"));

    let mut policy = Policy::load(&root);

    // Initially, file.special is hidden
    assert!(policy.is_hidden(&root.join("removable").join("file.special")));

    // Remove the subdir .agentignore
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::fs::remove_file(root.join("removable").join(".agentignore")).unwrap();

    // Hot reload should detect the removal
    assert!(policy.check_and_reload());

    // Now file.special should be visible (only root rules apply, which hide *.log)
    assert!(!policy.is_hidden(&root.join("removable").join("file.special")));
}
