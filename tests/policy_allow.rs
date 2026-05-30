use agentignore::fs::Policy;

mod common;

#[test]
fn policy_agentallow_hides_itself() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");
    common::touch(&root.join(".agentallow"));

    let policy = Policy::load(&root);
    // `.agentallow` itself should always be hidden
    assert!(policy.is_hidden(&root.join(".agentallow")));
}

#[test]
fn allowlist_pid_entry_matches_correct_pid() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "secret.txt\n");
    common::touch(&root.join("secret.txt"));

    let comm = std::fs::read_to_string("/proc/self/comm").unwrap();
    common::make_agentallow(&root, comm.trim());

    let policy = Policy::load(&root);
    let pid = std::process::id() as u32;

    // Current process should be allowed
    assert!(policy.is_allowed_raw(&root.join("secret.txt"), pid));
    // Non-existent PID should not be allowed
    assert!(!policy.is_allowed_raw(&root.join("secret.txt"), 99999));
}

#[test]
fn allowlist_process_name_entry_matches_current_process() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "secret.txt\n");
    common::touch(&root.join("secret.txt"));

    let comm = std::fs::read_to_string("/proc/self/comm").unwrap();
    common::make_agentallow(&root, comm.trim());

    let policy = Policy::load(&root);
    let pid = std::process::id() as u32;

    // Current process name should match
    assert!(policy.is_allowed_raw(&root.join("secret.txt"), pid));
    // Non-existent PID should not match
    assert!(!policy.is_allowed_raw(&root.join("secret.txt"), 99999));
}
