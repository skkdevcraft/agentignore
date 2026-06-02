use agentignore::fs::{CascadingAllowList, Policy};

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

mod common;

fn proc_comm() -> String {
    std::fs::read_to_string("/proc/self/comm")
        .unwrap()
        .trim()
        .to_string()
}

fn current_pid() -> u32 {
    std::process::id()
}

fn make_agentallow_in_dir(dir: &Path, content: &str) {
    fs::write(dir.join(".agentallow"), content).unwrap();
}

// ──────────────────────────────────────────────
//  Cascading .agentallow tests
// ──────────────────────────────────────────────

#[test]
fn cascading_agentallow_empty_root_but_subdir_has_rules() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    // Root has NO .agentallow
    // Subdir allows current process
    common::mkdirp(&root.join("subdir"));
    make_agentallow_in_dir(&root.join("subdir"), &format!("{comm}\n"));

    common::touch(&root.join("root_file.txt"));
    common::touch(&root.join("subdir").join("sub_file.txt"));

    let allow_list = CascadingAllowList::load(&root);

    // Root path NOT allowed (no .agentallow at root)
    assert!(!allow_list.is_allowed(&root.join("root_file.txt"), pid));

    // Subdir path IS allowed (has its own .agentallow)
    assert!(allow_list.is_allowed(&root.join("subdir").join("sub_file.txt"), pid));

    // has_any_entries returns true (subdir has entries)
    assert!(allow_list.has_any_entries());

    // Root alone has no entries
    let empty_root = CascadingAllowList::load(&root);
    assert!(empty_root.has_any_entries());
}

#[test]
fn cascading_agentallow_gid_across_levels() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    common::mkdirp(&root.join("services").join("db"));

    // Root: allow "npm" (or-child)
    common::make_agentallow(&root, "npm\n");
    // services/: allow current process
    make_agentallow_in_dir(&root.join("services"), &format!("{comm}\n"));
    // services/db/: allow "postgres" exact
    make_agentallow_in_dir(&root.join("services").join("db"), "postgres!\n");

    common::touch(&root.join("services").join("db").join("data.sql"));

    let allow_list = CascadingAllowList::load(&root);

    // Current process allowed at deepest level (inherited from services/)
    assert!(allow_list.is_allowed(&root.join("services").join("db").join("data.sql"), pid,));

    // Root level entry (npm) also propagates
    assert!(allow_list.has_any_entries());

    // Non-matching PID not allowed
    assert!(!allow_list.is_allowed(&root.join("services").join("db").join("data.sql"), 99999));
}

#[test]
fn cascading_agentallow_hot_reload_detects_new_subdir_allow() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    // Root allows current process
    common::make_agentallow(&root, &format!("{comm}\n"));
    common::mkdirp(&root.join("dynamic"));

    let mut policy = Policy::load(&root);

    common::touch(&root.join("dynamic").join("file.txt"));

    // Subdir inherits root allow
    assert!(policy.is_allowed_raw(&root.join("dynamic").join("file.txt"), pid));

    // Create .agentallow in subdir
    std::thread::sleep(std::time::Duration::from_millis(15));
    make_agentallow_in_dir(&root.join("dynamic"), "specific_tool\n");

    // Hot reload picks it up
    assert!(policy.check_and_reload());

    // Current process still allowed (root allow inherited)
    assert!(policy.is_allowed_raw(&root.join("dynamic").join("file.txt"), pid));

    // Both root and subdir allow entries present
    let allow_list = CascadingAllowList::load(&root);
    assert!(allow_list.has_any_entries());
    // Current process still allowed (inherited from root + new subdir allow)
    assert!(allow_list.is_allowed(&root.join("dynamic").join("file.txt"), pid));
}

#[test]
fn cascading_agentallow_hot_reload_detects_removed_subdir_allow() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    common::mkdirp(&root.join("removable"));
    // Subdir allows current process
    make_agentallow_in_dir(&root.join("removable"), &format!("{comm}\n"));

    let mut policy = Policy::load(&root);

    common::touch(&root.join("removable").join("file.txt"));

    // Initially, current process is allowed in subdir
    assert!(policy.is_allowed_raw(&root.join("removable").join("file.txt"), pid));

    // Remove the subdir .agentallow
    std::thread::sleep(std::time::Duration::from_millis(15));
    fs::remove_file(root.join("removable").join(".agentallow")).unwrap();

    // Hot reload detects the removal
    assert!(policy.check_and_reload());

    // Current process NO longer allowed (root has no allow, subdir allow removed)
    assert!(!policy.is_allowed_raw(&root.join("removable").join("file.txt"), pid));
}

#[test]
fn cascading_agentallow_multiple_rules_in_chain() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    // Chain: root → a → b → c
    common::mkdirp(&root.join("a").join("b").join("c"));

    // Each level adds its own .agentallow
    common::make_agentallow(&root, &format!("{comm}\n")); // root: current process
    make_agentallow_in_dir(&root.join("a"), &format!("{comm}\nnpm\n")); // a: current + npm
    make_agentallow_in_dir(&root.join("a").join("b"), "docker\n"); // b: docker
    make_agentallow_in_dir(&root.join("a").join("b").join("c"), "git\n"); // c: git

    common::touch(&root.join("a").join("b").join("c").join("file.txt"));

    let allow_list = CascadingAllowList::load(&root);

    // Current process allowed at deepest level (inherited from root and a/)
    assert!(allow_list.is_allowed(&root.join("a").join("b").join("c").join("file.txt"), pid,));

    // Non-matching PID not allowed
    assert!(!allow_list.is_allowed(&root.join("a").join("b").join("c").join("file.txt"), 99999,));
}

#[test]
fn cascading_agentallow_root_and_subdir_combined_with_agentignore_cascade() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    // Root hides *.log, allows current process
    common::make_agentignore(&root, "*.log\n");
    common::make_agentallow(&root, &format!("{comm}\n"));

    common::mkdirp(&root.join("subdir"));
    // Subdir additionally hides *.tmp
    fs::write(root.join("subdir").join(".agentignore"), "*.tmp\n").unwrap();
    // Subdir allows another process (not current)
    make_agentallow_in_dir(&root.join("subdir"), "another_daemon\n");

    common::touch(&root.join("app.log"));
    common::touch(&root.join("subdir").join("server.log"));
    common::touch(&root.join("subdir").join("cache.tmp"));

    let policy = Policy::load(&root);

    // Root agentignore hides root's .log
    assert!(policy.is_hidden(&root.join("app.log")));
    // Current process is allowed at root
    assert!(policy.is_allowed_raw(&root.join("app.log"), pid));

    // Subdir inherits root agentignore (hides *.log)
    assert!(policy.is_hidden(&root.join("subdir").join("server.log")));
    // Subdir adds its own *.tmp
    assert!(policy.is_hidden(&root.join("subdir").join("cache.tmp")));

    // Current process is allowed in subdir (inherited from root's allow)
    assert!(policy.is_allowed_raw(&root.join("subdir").join("server.log"), pid));
    assert!(policy.is_allowed_raw(&root.join("subdir").join("cache.tmp"), pid));

    // Non-matching PID not allowed
    assert!(!policy.is_allowed_raw(&root.join("app.log"), 99999));
    assert!(!policy.is_allowed_raw(&root.join("subdir").join("server.log"), 99999));
}

#[test]
fn cascading_agentallow_sibling_directories_have_different_rules() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    common::mkdirp(&root.join("frontend"));
    common::mkdirp(&root.join("backend"));

    // frontend allows current process
    make_agentallow_in_dir(&root.join("frontend"), &format!("{comm}\n"));
    // backend allows a different process
    make_agentallow_in_dir(&root.join("backend"), "backend_daemon\n");

    common::touch(&root.join("frontend").join("app.js"));
    common::touch(&root.join("backend").join("server.py"));

    let allow_list = CascadingAllowList::load(&root);

    // Current process allowed in frontend
    assert!(allow_list.is_allowed(&root.join("frontend").join("app.js"), pid));
    // Current process NOT allowed in backend
    assert!(!allow_list.is_allowed(&root.join("backend").join("server.py"), pid));
    // Non-matching PID not allowed anywhere
    assert!(!allow_list.is_allowed(&root.join("frontend").join("app.js"), 99999));
    assert!(!allow_list.is_allowed(&root.join("backend").join("server.py"), 99999));
}

#[test]
fn cascading_agentallow_subdir_adds_specific_rules() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    // Root allows current process
    common::make_agentallow(&root, &format!("{comm}\n"));

    common::mkdirp(&root.join("subdir"));
    // Subdir has its own allow for a different process
    make_agentallow_in_dir(&root.join("subdir"), "subdir_tool\n");

    common::touch(&root.join("subdir").join("file.txt"));

    let allow_list = CascadingAllowList::load(&root);

    // Current process is allowed in subdir (inherited from root)
    assert!(allow_list.is_allowed(&root.join("subdir").join("file.txt"), pid));

    // Subdir's own entry registered
    assert!(allow_list.has_any_entries());
}

#[test]
fn cascading_agentallow_subdir_inherits_root_allows() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    // Root allows the current process
    common::make_agentallow(&root, &format!("{comm}\n"));

    common::mkdirp(&root.join("subdir"));
    common::touch(&root.join("subdir").join("file.txt"));

    let allow_list = CascadingAllowList::load(&root);

    // Root path allows
    assert!(allow_list.is_allowed(&root.join("file.txt"), pid));
    // Subdir path inherits root allow
    assert!(allow_list.is_allowed(&root.join("subdir").join("file.txt"), pid));

    // Non-matching PID not allowed
    assert!(!allow_list.is_allowed(&root.join("subdir").join("file.txt"), 99999));
}

#[test]
fn cascading_agentallow_with_agentfs_integration_lookup() {
    let (_dir, root) = common::test_dir();
    let pid = current_pid();
    let comm = proc_comm();

    // Agentignore hides *.log everywhere
    common::make_agentignore(&root, "*.log\n");
    // Root allows current process
    common::make_agentallow(&root, &format!("{comm}\n"));

    common::mkdirp(&root.join("project"));
    // Subdir adds its own .agentignore and .agentallow
    fs::write(root.join("project").join(".agentignore"), "*.secret\n").unwrap();
    make_agentallow_in_dir(&root.join("project"), "project_specific\n");

    common::touch(&root.join("app.log"));
    common::touch(&root.join("project").join("server.log"));
    common::touch(&root.join("project").join("key.secret"));
    common::touch(&root.join("project").join("readme.md"));

    let fs = agentignore::fs::AgentFS::new(root.clone());

    // .log files hidden (with no request context, no allow-bypass)
    assert!(
        fs.lookup_child(fuser::INodeNo::ROOT, OsStr::new("app.log"), None)
            .is_none()
    );

    // project directory visible
    assert!(
        fs.lookup_child(fuser::INodeNo::ROOT, OsStr::new("project"), None)
            .is_some()
    );

    let (_project_path, _, project_ino) = fs
        .lookup_child(fuser::INodeNo::ROOT, OsStr::new("project"), None)
        .unwrap();

    // Inside project: server.log hidden (root's *.log cascades)
    assert!(
        fs.lookup_child(fuser::INodeNo(project_ino), OsStr::new("server.log"), None)
            .is_none()
    );

    // key.secret hidden (project's *.secret)
    assert!(
        fs.lookup_child(fuser::INodeNo(project_ino), OsStr::new("key.secret"), None)
            .is_none()
    );

    // readme.md visible
    assert!(
        fs.lookup_child(fuser::INodeNo(project_ino), OsStr::new("readme.md"), None)
            .is_some()
    );

    // Verify cascading allow via Policy
    let policy = fs.policy_read();

    // Current process allowed at root
    assert!(policy.is_allowed_raw(&root.join("app.log"), pid));
    // Current process allowed in subdir (inherited from root)
    assert!(policy.is_allowed_raw(&root.join("project").join("server.log"), pid));
    // Current process allowed for project's own secret file too
    assert!(policy.is_allowed_raw(&root.join("project").join("key.secret"), pid));

    // Non-matching PID not allowed
    assert!(!policy.is_allowed_raw(&root.join("project").join("readme.md"), 99999));
}
