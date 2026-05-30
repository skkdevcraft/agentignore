use agentfs::fs::{AgentFS, Policy};

use fuser::INodeNo;

use std::ffi::OsStr;
use std::fs;

mod common;

#[test]
fn cascading_agentignore_subdir_inherits_root_rules() {
    let (_dir, root) = common::test_dir();
    // Root .agentignore hides *.log everywhere
    common::make_agentignore(&root, "*.log\n");

    // Create files in root and subdirectory
    common::touch(&root.join("root.log"));
    common::mkdirp(&root.join("subdir"));
    common::touch(&root.join("subdir").join("sub.log"));
    common::touch(&root.join("subdir").join("sub.txt"));

    let policy = Policy::load(&root);

    // Root rules apply in root
    assert!(policy.is_hidden(&root.join("root.log")));

    // Root rules cascade to subdirectory
    assert!(policy.is_hidden(&root.join("subdir").join("sub.log")));
    assert!(!policy.is_hidden(&root.join("subdir").join("sub.txt")));
}

#[test]
fn cascading_agentignore_subdir_overrides_root() {
    let (_dir, root) = common::test_dir();
    // Root hides all .tmp files
    common::make_agentignore(&root, "*.tmp\n");

    common::mkdirp(&root.join("subdir"));

    // Subdir has its own .agentignore that negates the root rule
    fs::write(root.join("subdir").join(".agentignore"), "!important.tmp\n").unwrap();

    common::touch(&root.join("root.tmp"));
    common::touch(&root.join("subdir").join("sub.tmp"));
    common::touch(&root.join("subdir").join("important.tmp"));

    // Reload policy to pick up subdir config
    let policy = Policy::load(&root);

    // Root rule still applies in root
    assert!(policy.is_hidden(&root.join("root.tmp")));

    // Root rule applies in subdir for unmatched files
    assert!(policy.is_hidden(&root.join("subdir").join("sub.tmp")));

    // Subdir override un-hides important.tmp
    assert!(!policy.is_hidden(&root.join("subdir").join("important.tmp")));
}

#[test]
fn cascading_agentignore_subdir_adds_new_rules() {
    let (_dir, root) = common::test_dir();
    // Root hides *.log
    common::make_agentignore(&root, "*.log\n");

    common::mkdirp(&root.join("src"));

    // Subdir additionally hides *.generated.rs
    fs::write(root.join("src").join(".agentignore"), "*.generated.rs\n").unwrap();

    common::touch(&root.join("app.log"));
    common::touch(&root.join("src").join("server.log"));
    common::touch(&root.join("src").join("lib.rs"));
    common::touch(&root.join("src").join("parser.generated.rs"));

    let policy = Policy::load(&root);

    // Root rules still apply everywhere
    assert!(policy.is_hidden(&root.join("app.log")));
    assert!(policy.is_hidden(&root.join("src").join("server.log")));

    // Subdir rules apply only in subdir
    assert!(!policy.is_hidden(&root.join("src").join("lib.rs")));
    assert!(policy.is_hidden(&root.join("src").join("parser.generated.rs")));

    // Files matching subdir pattern but in root are NOT hidden
    common::touch(&root.join("root.generated.rs"));
    assert!(!policy.is_hidden(&root.join("root.generated.rs")));
}

#[test]
fn cascading_agentignore_nested_subdirs_chain_rules() {
    let (_dir, root) = common::test_dir();
    // Root: hide *.log
    common::make_agentignore(&root, "*.log\n");

    common::mkdirp(&root.join("a").join("b").join("c"));

    // Level a: hide *.tmp
    fs::write(root.join("a").join(".agentignore"), "*.tmp\n").unwrap();

    // Level a/b: hide *.cache
    fs::write(root.join("a").join("b").join(".agentignore"), "*.cache\n").unwrap();

    // Create test files at each level
    common::touch(&root.join("root.log"));
    common::touch(&root.join("a").join("a.log"));
    common::touch(&root.join("a").join("a.tmp"));
    common::touch(&root.join("a").join("b").join("b.log"));
    common::touch(&root.join("a").join("b").join("b.tmp"));
    common::touch(&root.join("a").join("b").join("b.cache"));
    common::touch(&root.join("a").join("b").join("c").join("c.log"));
    common::touch(&root.join("a").join("b").join("c").join("c.tmp"));
    common::touch(&root.join("a").join("b").join("c").join("c.cache"));
    common::touch(&root.join("a").join("b").join("c").join("c.txt"));

    let policy = Policy::load(&root);

    // Root level: only *.log hidden
    assert!(policy.is_hidden(&root.join("root.log")));

    // Level a: *.log (from root) + *.tmp (from a) hidden
    assert!(policy.is_hidden(&root.join("a").join("a.log")));
    assert!(policy.is_hidden(&root.join("a").join("a.tmp")));

    // Level a/b: *.log + *.tmp + *.cache hidden
    assert!(policy.is_hidden(&root.join("a").join("b").join("b.log")));
    assert!(policy.is_hidden(&root.join("a").join("b").join("b.tmp")));
    assert!(policy.is_hidden(&root.join("a").join("b").join("b.cache")));

    // Level a/b/c: *.log + *.tmp + *.cache hidden, c.txt visible
    assert!(policy.is_hidden(&root.join("a").join("b").join("c").join("c.log")));
    assert!(policy.is_hidden(&root.join("a").join("b").join("c").join("c.tmp")));
    assert!(policy.is_hidden(&root.join("a").join("b").join("c").join("c.cache")));
    assert!(!policy.is_hidden(&root.join("a").join("b").join("c").join("c.txt")));
}

#[test]
fn cascading_agentignore_directory_specific_patterns() {
    let (_dir, root) = common::test_dir();
    // Root: ignore build/ directory
    common::make_agentignore(&root, "build/\n");

    common::mkdirp(&root.join("frontend").join("build"));
    common::mkdirp(&root.join("backend").join("build"));

    // Backend has its own rules that un-ignore build/
    fs::write(root.join("backend").join(".agentignore"), "!build/\n").unwrap();

    let policy = Policy::load(&root);

    // Root build directory is hidden
    common::mkdirp(&root.join("build"));
    assert!(policy.is_hidden(&root.join("build")));

    // Frontend build inherits root rule
    assert!(policy.is_hidden(&root.join("frontend").join("build")));

    // Backend build is un-hidden by local override
    assert!(!policy.is_hidden(&root.join("backend").join("build")));
}

#[test]
fn cascading_agentignore_empty_subdir_config_no_effect() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");

    common::mkdirp(&root.join("empty_config"));
    // Create an empty .agentignore in subdir
    fs::write(root.join("empty_config").join(".agentignore"), "").unwrap();

    common::touch(&root.join("empty_config").join("test.log"));
    common::touch(&root.join("empty_config").join("test.txt"));

    let policy = Policy::load(&root);

    // Empty subdir config shouldn't affect root rules
    assert!(policy.is_hidden(&root.join("empty_config").join("test.log")));
    assert!(!policy.is_hidden(&root.join("empty_config").join("test.txt")));
}

#[test]
fn cascading_agentignore_comments_only_subdir_config() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");

    common::mkdirp(&root.join("comments_only"));
    // Create a .agentignore with only comments
    fs::write(
        root.join("comments_only").join(".agentignore"),
        "# This is a comment\n# Another comment\n",
    )
    .unwrap();

    common::touch(&root.join("comments_only").join("test.log"));
    common::touch(&root.join("comments_only").join("test.txt"));

    let policy = Policy::load(&root);

    // Comment-only subdir config should act like no config (inherit root)
    assert!(policy.is_hidden(&root.join("comments_only").join("test.log")));
    assert!(!policy.is_hidden(&root.join("comments_only").join("test.txt")));
}

#[test]
fn cascading_agentignore_subdir_hides_its_own_config() {
    let (_dir, root) = common::test_dir();

    common::mkdirp(&root.join("project"));

    // Create .agentignore in subdir
    fs::write(root.join("project").join(".agentignore"), "*.secret\n").unwrap();

    common::touch(&root.join("project").join("data.secret"));

    let policy = Policy::load(&root);

    // The .agentignore file in subdir should itself be hidden
    assert!(policy.is_hidden(&root.join("project").join(".agentignore")));
    // Files matching the pattern should be hidden
    assert!(policy.is_hidden(&root.join("project").join("data.secret")));
}

#[test]
fn cascading_agentignore_deeply_nested_subdir_with_no_config_uses_nearest_ancestor() {
    let (_dir, root) = common::test_dir();

    common::mkdirp(&root.join("x").join("y").join("z"));

    // Only middle level has config
    fs::write(root.join("x").join("y").join(".agentignore"), "*.mid\n").unwrap();

    common::touch(&root.join("x").join("file.txt"));
    common::touch(&root.join("x").join("y").join("file.mid"));
    common::touch(&root.join("x").join("y").join("z").join("file.mid"));
    common::touch(&root.join("x").join("y").join("z").join("file.txt"));

    let policy = Policy::load(&root);

    // Level x: no local config, no root config → everything visible
    assert!(!policy.is_hidden(&root.join("x").join("file.txt")));

    // Level x/y: has config hiding *.mid
    assert!(policy.is_hidden(&root.join("x").join("y").join("file.mid")));

    // Level x/y/z: no config, but inherits from x/y
    assert!(policy.is_hidden(&root.join("x").join("y").join("z").join("file.mid")));
    assert!(!policy.is_hidden(&root.join("x").join("y").join("z").join("file.txt")));
}

#[test]
fn cascading_agentignore_with_agentfs_integration() {
    let (_dir, root) = common::test_dir();

    // Root hides *.log
    common::make_agentignore(&root, "*.log\n");

    common::mkdirp(&root.join("logs"));

    // Subdir additionally hides *.trace
    fs::write(root.join("logs").join(".agentignore"), "*.trace\n").unwrap();

    common::touch(&root.join("root.log"));
    common::touch(&root.join("logs").join("server.log"));
    common::touch(&root.join("logs").join("debug.trace"));
    common::touch(&root.join("logs").join("info.txt"));

    let fs = AgentFS::new(root.clone());

    // Test through AgentFS
    assert!(fs.is_hidden(&root.join("root.log")));
    assert!(fs.is_hidden(&root.join("logs").join("server.log")));
    assert!(fs.is_hidden(&root.join("logs").join("debug.trace")));
    assert!(!fs.is_hidden(&root.join("logs").join("info.txt")));

    // Test that lookup fails for hidden files
    assert!(
        fs.lookup_child(INodeNo::ROOT, OsStr::new("root.log"), None)
            .is_none()
    );

    assert!(
        fs.lookup_child(INodeNo::ROOT, OsStr::new("logs"), None)
            .is_some()
    );

    let (logs_path, logs_ino) = fs
        .lookup_child(INodeNo::ROOT, OsStr::new("logs"), None)
        .unwrap();
    assert!(logs_path.ends_with("logs"));

    // Hidden inside logs/
    assert!(
        fs.lookup_child(INodeNo(logs_ino), OsStr::new("debug.trace"), None)
            .is_none()
    );

    // Visible inside logs/
    assert!(
        fs.lookup_child(INodeNo(logs_ino), OsStr::new("info.txt"), None)
            .is_some()
    );
}

#[test]
fn cascading_agentignore_preserves_hiding_of_config_files_in_subdirs() {
    let (_dir, root) = common::test_dir();

    common::mkdirp(&root.join("a").join("b"));

    fs::write(root.join("a").join(".agentignore"), "*.secret\n").unwrap();

    let policy = Policy::load(&root);

    // .agentignore files should be hidden regardless of location
    assert!(policy.is_hidden(&root.join(".agentignore")));
    assert!(policy.is_hidden(&root.join("a").join(".agentignore")));

    // .agentallow should also be hidden
    common::touch(&root.join(".agentallow"));
    common::touch(&root.join("a").join(".agentallow"));
    assert!(policy.is_hidden(&root.join(".agentallow")));
    assert!(policy.is_hidden(&root.join("a").join(".agentallow")));
}
