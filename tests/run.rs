//! Integration tests for `agentignore run`.
//!
//! These tests exercise the CLI binary as a subprocess to verify cleanup
//! behaviour (mountpoint removal) in various scenarios.

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper: path to the `agentignore` binary (set by `cargo test`).
fn agentignore_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agentignore"))
}

/// Helper: count leaked mountpoint directories under `/tmp/agentignore/` that
/// start with the given `source_name`.
fn leaked_mountpoints(source_name: &str) -> Vec<PathBuf> {
    let tmp = std::env::temp_dir().join("agentignore");
    if !tmp.exists() {
        return Vec::new();
    }

    let mut leaked = Vec::new();
    for entry in std::fs::read_dir(&tmp).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if let Some(name) = name.to_str() {
            if name.starts_with(source_name) {
                // Exclude entries that are not actual directories created by us
                if entry.path().is_dir() {
                    leaked.push(entry.path());
                }
            }
        }
    }
    leaked
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn bad_command_cleans_up_mountpoint() {
    /// The name of the nonexistent command we pass.
    const BAD_CMD: &str = "nonexistent-cmd-xyzzy-99999";
    let source = TempDir::new().unwrap();
    let source_path = source.path();

    let output = Command::new(agentignore_bin())
        .args(["run", "-s", &source_path.to_string_lossy(), BAD_CMD])
        .output()
        .expect("failed to execute agentignore run");

    // The process must exit with code 1 when the command isn't found.
    assert!(
        !output.status.success(),
        "expected exit code 1 (not found), got success"
    );

    // Verify the error message appears on stderr.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Failed to execute command"),
        "expected 'Failed to execute command' on stderr, got: {stderr}"
    );
    assert!(
        stderr.contains(BAD_CMD),
        "expected bad command name on stderr, got: {stderr}"
    );

    // Core assertion: no mountpoint directory leaked.
    let source_name = source_path.file_name().unwrap().to_str().unwrap();
    let leaked = leaked_mountpoints(source_name);
    assert!(
        leaked.is_empty(),
        "Mountpoint directory was NOT cleaned up after `agentignore run` with \
         a non-existent command.\n  Leaked directories:\n    {}\n\n\
         This means cleanup (unmount + remove_dir) did not run or failed.",
        leaked
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n    "),
    );
}

#[test]
fn bad_command_stderr_includes_error() {
    let source = TempDir::new().unwrap();
    let source_path = source.path();

    let output = Command::new(agentignore_bin())
        .args([
            "run",
            "-s",
            &source_path.to_string_lossy(),
            "non-existent-command",
        ])
        .output()
        .expect("failed to execute agentignore run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No such file or directory")
            || stderr.contains("not found")
            || stderr.contains("Permission denied"),
        "expected an OS-level error message on stderr, got: {stderr}"
    );
}

/// Verify that with a valid command (like `true`), the mountpoint is still
/// cleaned up and the process exits with code 0.
#[test]
fn valid_command_cleans_up_mountpoint() {
    let source = TempDir::new().unwrap();
    let source_path = source.path();

    let output = Command::new(agentignore_bin())
        .args(["run", "-s", &source_path.to_string_lossy(), "true"])
        .output()
        .expect("failed to execute agentignore run");

    // `true` should exit 0.
    assert!(
        output.status.success(),
        "expected exit code 0 for 'true', got {:?}",
        output.status.code()
    );

    // Verify cleanup.
    let source_name = source_path.file_name().unwrap().to_str().unwrap();
    let leaked = leaked_mountpoints(source_name);
    assert!(
        leaked.is_empty(),
        "Mountpoint leaked after valid command: {leaked:?}"
    );
}
