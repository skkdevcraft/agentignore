//! `agentignore init` — Create example `.agentignore` and `.agentallow` files.

use crate::cmd::common::{write_agentallow, write_agentignore};
use std::path::PathBuf;

/// Handle `agentignore init [folder]`.
///
/// Creates example `.agentignore` and `.agentallow` files in the specified
/// directory (or the current directory if none is given).
pub fn init(folder: Option<PathBuf>) {
    let dir = folder.unwrap_or_else(|| std::env::current_dir().unwrap());
    std::fs::create_dir_all(&dir).expect("failed to create directory");

    write_agentignore(&dir);
    write_agentallow(&dir);

    println!("✓ Created .agentignore and .agentallow in {:?}", dir);
}
