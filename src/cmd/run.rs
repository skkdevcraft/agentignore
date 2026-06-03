//! `agentignore run` — Mount, run a command, then unmount (ephemeral).

use crate::cmd::common::{create_temp_mountpoint, setup_signal_handler, unmount_internal};
use agentignore::fs::AgentFS;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

/// Handle `agentignore run [command...]` with an optional `--source <path>`
/// and `--show-config-files`.
///
/// Sets up a temporary mountpoint, spawns a command inside it, then unmounts
/// and exits with the command's exit code.
///
/// The first element of `command` is the program to run; the rest are its arguments.
pub fn run(command: Vec<String>, source: Option<PathBuf>, show_config_files: bool) {
    // Verify FUSE prerequisites before attempting to mount
    crate::cmd::doctor::check_prerequisites(true);

    let source = source.unwrap_or_else(|| std::env::current_dir().unwrap());
    let source = source.canonicalize().expect("source path must exist");
    let mountpoint = create_temp_mountpoint(&source);

    // Set up signal handler for the Run command
    setup_signal_handler(Some(mountpoint.clone()), true);

    println!("Mounting {:?} → {:?}", source, mountpoint);

    // Mount in a separate thread since fuser::mount2 blocks
    let fs = AgentFS::with_config(source.clone(), None, show_config_files);
    let mp_clone = mountpoint.clone();
    let mount_handle = std::thread::spawn(move || {
        fuser::mount2(fs, &mp_clone, &fuser::Config::default()).expect("mount failed");
    });

    // Give FUSE a moment to start
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Split command into program and arguments
    let program = &command[0];
    let args = &command[1..];
    let command_string = command.join(" "); // For error reporting

    // Run the command and capture the result
    let result = ProcessCommand::new(program)
        .args(args)
        .current_dir(&mountpoint)
        .env("PWD", &mountpoint)
        .status();

    // --- Cleanup (always performed, even if command fails) ---
    unmount_internal(&mountpoint);

    // Remove temp directory
    if let Err(e) = std::fs::remove_dir(&mountpoint) {
        eprintln!(
            "Warning: failed to remove temporary directory {:?}: {}",
            mountpoint, e
        );
    }

    // Wait for mount thread; ignore errors as the thread may have already panicked
    let _ = mount_handle.join();

    // --- Handle command result after cleanup ---
    match result {
        Ok(status) => {
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("Failed to execute command '{}': {}", command_string, e);
            std::process::exit(1);
        }
    }
}
