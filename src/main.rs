//! Entry point for `agentignore` — a policy-filtered FUSE filesystem for AI agents.
//!
//! This binary provides CLI subcommands to mount, unmount, check, init, and run
//! commands against a filtered view of a filesystem, where files matching
//! patterns in `.agentignore` are hidden from the agent.
//!
//! # Overview
//!
//! - **`run`** — Mount, run a command, then unmount (ephemeral).
//! - **`init`** — Create example `.agentignore` and `.agentallow` files.
//! - **`explain`** — Show whether a path is hidden and why.
//! - **`mount`** — Mount a filtered view of `<source>` at `<mountpoint>`.
//! - **`unmount`** — Unmount an existing AgentIgnore mountpoint.
//! - **`check`** — Validate the `.agentignore` in the current directory.

use clap::Parser;
use cmd::args::{Args, Command};

mod cmd;

/// The `agentignore` binary entry point.
///
/// Initialises tracing (logging), parses CLI arguments via [`clap`], and
/// dispatches to the appropriate subcommand handler.
fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    match args.command {
        Command::Run {
            command,
            source,
            show_config_files,
        } => cmd::run::run(command, source, show_config_files),
        Command::Mount {
            source,
            mountpoint,
            no_dashboard,
            show_config_files,
        } => cmd::mount::mount(source, mountpoint, no_dashboard, show_config_files),
        Command::Unmount { mountpoint } => cmd::unmount::unmount(mountpoint),
        Command::Init { folder } => cmd::init::init(folder),
        Command::Check => cmd::check::check(),
        Command::Doctor => cmd::doctor::doctor(),
        Command::Explain { path } => cmd::explain::explain(path),
    }
}
