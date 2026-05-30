//! CLI argument parsing for `agentfs` subcommands.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "agentfs",
    about = "Policy-filtered FUSE filesystem for AI agents",
    version,
    after_help = "EXAMPLES:\n    agentfs init              Create example .agentignore and .agentallow files in the current folder\n    agentfs run bash          Mount the current folder and run bash in it\n    agentfs run pi            Mount the current folder and run pi coding agent in it\n    agentfs run -- bash -c 'ls && cat .env'  Run a multi-word command inside the filtered view\n    agentfs explain /etc      Show why /etc would be hidden"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Mount a filtered view, run a command, then unmount
    Run {
        /// The command to run, including its arguments
        #[arg(trailing_var_arg = true, required = true, num_args = 1..)]
        command: Vec<String>,
        /// Optional source directory to filter (defaults to current directory)
        #[arg(short = 's', long = "source")]
        source: Option<PathBuf>,
    },
    /// Create example .agentignore and .agentallow files in the specified directory
    Init {
        /// Target directory (defaults to current directory)
        folder: Option<PathBuf>,
    },
    /// Explain whether a path would be hidden and why
    Explain { path: PathBuf },
    /// Mount a filtered view of <source> at <mountpoint>
    Mount {
        source: PathBuf,
        mountpoint: PathBuf,
        /// Disable the live dashboard (no stats collection, no rendering)
        #[arg(long = "no-dashboard", short = 'D', default_value_t = false)]
        no_dashboard: bool,
    },
    /// Unmount an AgentFS mountpoint
    Unmount { mountpoint: PathBuf },
    /// Validate the .agentignore in the current directory
    Check,
}
