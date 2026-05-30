use agentfs::fs::Policy;

mod common;

#[test]
fn policy_hides_agentignore_itself() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");
    common::touch(&root.join(".agentignore"));

    let policy = Policy::load(&root);
    assert!(policy.is_hidden(&root.join(".agentignore")));
}

#[test]
fn policy_hides_matched_file() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");
    common::touch(&root.join("server.log"));

    let policy = Policy::load(&root);
    assert!(policy.is_hidden(&root.join("server.log")));
}

#[test]
fn policy_does_not_hide_unmatched_file() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.log\n");
    common::touch(&root.join("main.rs"));

    let policy = Policy::load(&root);
    assert!(!policy.is_hidden(&root.join("main.rs")));
}

#[test]
fn policy_with_no_agentignore_hides_nothing() {
    let (_dir, root) = common::test_dir();
    common::touch(&root.join("whatever.txt"));

    let policy = Policy::load(&root);
    assert!(!policy.is_hidden(&root.join("whatever.txt")));
}

#[test]
fn policy_hides_directory_by_pattern() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "target/\n");
    common::mkdirp(&root.join("target"));

    let policy = Policy::load(&root);
    assert!(policy.is_hidden(&root.join("target")));
}

#[test]
fn policy_with_multiple_patterns() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "*.o\n*.a\ntmp/\n");
    common::touch(&root.join("foo.o"));
    common::touch(&root.join("bar.a"));
    common::touch(&root.join("baz.rs"));
    common::mkdirp(&root.join("tmp"));

    let policy = Policy::load(&root);
    assert!(policy.is_hidden(&root.join("foo.o")));
    assert!(policy.is_hidden(&root.join("bar.a")));
    assert!(policy.is_hidden(&root.join("tmp")));
    assert!(!policy.is_hidden(&root.join("baz.rs")));
}

#[test]
fn policy_empty_agentignore_hides_nothing() {
    let (_dir, root) = common::test_dir();
    common::make_agentignore(&root, "");
    common::touch(&root.join("any.file"));

    let policy = Policy::load(&root);
    assert!(!policy.is_hidden(&root.join("any.file")));
}
