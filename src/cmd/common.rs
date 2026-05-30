//! Shared helper functions used by multiple agentignore subcommands.

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Create a temporary directory in the system temp directory to use as a
/// mountpoint. The directory name is based on the last component of the
/// source path. If that directory already exists, appends the process ID.
///
/// # Panics
///
/// Panics if the directory cannot be created.
pub fn create_temp_mountpoint(source: &Path) -> PathBuf {
    let base_name = source
        .file_name()
        .expect("source path must have a file name")
        .to_str()
        .expect("source path must be valid UTF-8");

    // Try the simple name first
    let dir = std::env::temp_dir().join(format!("agentignore/{}", base_name));

    // If it already exists, append the process ID
    if dir.exists() {
        let dir =
            std::env::temp_dir().join(format!("agentignore/{}-{}", base_name, std::process::id()));
        std::fs::create_dir_all(&dir).expect("failed to create temp mountpoint");
        dir
    } else {
        std::fs::create_dir_all(&dir).expect("failed to create temp mountpoint");
        dir
    }
}

/// Unmount a FUSE filesystem using `fusermount -u`.
///
/// Prints a warning to stderr if the unmount fails, but does not exit.
/// This is intended for cleanup in signal handlers and the `Run` subcommand,
/// where graceful degradation is preferred over hard failure.
pub fn unmount_internal(mountpoint: &PathBuf) {
    let status = ProcessCommand::new("fusermount")
        .args(["-u", mountpoint.to_str().unwrap()])
        .status()
        .expect("fusermount not found");
    if !status.success() {
        eprintln!("Warning: fusermount failed for {:?}", mountpoint);
    }
}

/// Register a SIGINT (Ctrl+C) handler that unmounts the filesystem and
/// optionally removes the mountpoint directory before exiting.
///
/// If `mountpoint` is `Some`, the signal handler will call
/// [`unmount_internal`] on it and, if `was_created` is true, remove the
/// directory. This ensures clean teardown even when the user hits Ctrl+C.
///
/// # Panics
///
/// Panics if `ctrlc::set_handler` fails (e.g. if a handler is already set).
pub fn setup_signal_handler(mountpoint: Option<PathBuf>, was_created: bool) {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        if let Some(ref mp) = mountpoint {
            eprintln!("\nReceived interrupt signal. Unmounting {:?}...", mp);
            unmount_internal(mp);
            if was_created && let Err(e) = std::fs::remove_dir(mp) {
                eprintln!("Warning: failed to remove mountpoint {:?}: {}", mp, e);
            }
        }
        std::process::exit(1);
    })
    .expect("failed to set signal handler");

    // Store running in a static to keep it alive
    std::mem::forget(running);
}

/// Build a template string for `.agentignore`.
///
/// If a `.gitignore` exists alongside the target file, its contents are
/// included and annotated as the starting set of ignore rules. Otherwise a
/// generic commented template is returned.
pub fn build_agentignore_template(dir: &Path) -> String {
    let gitignore_path = dir.join(".gitignore");

    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path).expect("failed to read .gitignore");
        format!(
            "\
# .agentignore — Files and patterns that are HIDDEN from the AI agent.
# Lines starting with `#` are comments.
# Syntax: standard gitignore pattern syntax (same as .gitignore).
#
# This file was initialized from your existing .gitignore.
# You may add or remove patterns to control what the agent can see.
#
{}",
            content
        )
    } else {
        "\
# .agentignore — Files and patterns that are HIDDEN from the AI agent.
# Lines starting with `#` are comments.
# Syntax: standard gitignore pattern syntax (same as .gitignore).
#
# Add patterns for files / directories that should be hidden from the agent.
#
# Examples:
#   .env                # Hidden: single file
#   secrets/            # Hidden: entire directory
#   *.log               # Hidden: all .log files
#   !important.log      # Visible: exception to the above
#
# By default, nothing is hidden. Uncomment or add patterns below:
# .env
# *.pem
# target/
# node_modules/
# dist/
# bin/
# obj/
"
        .to_string()
    }
}

/// Build a template string for `.agentallow`.
///
/// The `.agentallow` file defines a *process-based bypass* list: processes
/// matching any entry here can see hidden files in the AgentIgnore mount.
/// See the AllowList comment block in `fs.rs` for the full format spec.
pub fn build_agentallow_template() -> String {
    "\
# .agentallow — Process-based bypass rules
# Lines starting with `#` are comments. Empty lines are ignored.
#
# This file defines which processes can BYPASS the hiding rules of AgentIgnore.
# Each entry specifies a process name, cmdline pattern, or binary path whose
# filesystem requests will see hidden files.
#
# \u{2015}\u{2015} Process name / cmdline matching \u{2015}\u{2015}
#   Default (e.g., `node`):
#       Regex matched against the process comm name OR full cmdline.
#       Ancestors are also checked (walk up the parent chain).
#   Prefix `=` (e.g., `=npm`):
#       Exact string match against comm OR cmdline.
#       Ancestors are still checked.
#   Suffix `!` after name (e.g., `node!`, `=npm!`):
#       Match ONLY the exact process, NOT its ancestors.
#
# \u{2015}\u{2015} Binary path matching (entry starts with `/`) \u{2015}\u{2015}
#   Default (e.g., `/usr/bin/node`):
#       Literal path match; matches process at that path OR any ancestor.
#   Suffix `!` (e.g., `/usr/bin/node!`):
#       Match ONLY the process at that exact path, no ancestor walk.
#   Binary paths are always compared literally (not regex).
#
# \u{2015}\u{2015} Examples \u{2015}\u{2015}
#   node              # regex; matches `node`, `nodejs`, any cmdline
#                     # containing `node`, and any child processes
#   =npm              # exact; matches only processes whose comm or
#                     # cmdline equals `npm` (and their children)
#   ^npm run          # matches cmdline starting with `npm run` (and their children)
#                     # 
#   =bash!            # exact; matches only bash itself, not commands
#                     # run inside it
#   node!             # regex; matches processes whose comm/cmdline
#                     # matches /node/, but does NOT walk up parents
#   /usr/bin/node     # literal path, matches process or any child
#   /usr/bin/java!    # literal path, matches only that exact process
"
    .to_string()
}

/// Write the initial `.agentignore` file to the given directory.
pub fn write_agentignore(dir: &Path) {
    let path = dir.join(".agentignore");
    let content = build_agentignore_template(dir);
    std::fs::write(&path, content).expect("failed to write .agentignore");
}

/// Write the initial `.agentallow` file to the given directory.
pub fn write_agentallow(dir: &Path) {
    let path = dir.join(".agentallow");
    let content = build_agentallow_template();
    std::fs::write(&path, content).expect("failed to write .agentallow");
}
