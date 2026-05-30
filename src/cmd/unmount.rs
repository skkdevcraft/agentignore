//! `agentignore unmount` — Unmount an AgentIgnore mountpoint.

use std::path::PathBuf;

/// Handle `agentignore unmount <mountpoint>`.
pub fn unmount(mountpoint: PathBuf) {
    let status = std::process::Command::new("fusermount")
        .args(["-u", mountpoint.to_str().unwrap()])
        .status()
        .expect("fusermount not found");
    if !status.success() {
        eprintln!("fusermount failed");
        std::process::exit(1);
    }
}
