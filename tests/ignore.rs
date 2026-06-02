use ignore::gitignore::GitignoreBuilder;
use std::fs;
use tempfile::tempdir;

#[test]
fn hides_env_file() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join(".agentignore"), ".env\n").unwrap();
    fs::write(dir.path().join(".env"), "SECRET=123").unwrap();

    let mut builder = GitignoreBuilder::new(dir.path());
    assert!(builder.add(dir.path().join(".agentignore")).is_none());
    let matcher = builder.build().unwrap();

    let hidden = matcher
        .matched_path_or_any_parents(dir.path().join(".env"), false)
        .is_ignore();
    assert!(hidden);
}
