# agentignore

[![Crates.io](https://img.shields.io/crates/v/agentignore)](https://crates.io/crates/agentignore)
[![npm version](https://img.shields.io/npm/v/@xlansoftware/agentignore)](https://www.npmjs.com/package/@xlansoftware/agentignore)

Policy-Filtered Filesystem for AI Agents.

**agentignore** is a [FUSE](https://en.wikipedia.org/wiki/Filesystem_in_Userspace) filesystem that provides a **filtered virtual view** of a real directory tree. Files and directories matching rules in a `.agentignore` file are completely hidden from processes interacting with the mounted filesystem — they don't appear in `ls`, `find`, glob expansion, or direct access.

The primary use case is constraining autonomous AI/LLM agents that have shell access, so irrelevant files and directories (`node_modules/`, `.git/`, `target/`, `.env`, `secrets/`, etc.) genuinely appear non-existent to the agent while still allowing the agent to execute build and test tools.

All visible files behave as a transparent passthrough: reads and writes go directly to the real filesystem.

> **Note:** This is not a security boundary. AgentIgnore is designed to reduce LLM context by hiding irrelevant files, not to prevent a determined attacker from accessing them.

## Table of Contents

- [Quick Start](#quick-start)
- [Prerequisites for Running](#prerequisites-for-running)
  - [Linux (bare-metal)](#linux-bare-metal)
  - [Docker / Dev Container](#docker--dev-container)
  - [WSL2](#wsl2)
- [Usage](#usage)
  - [`init` — create policy files](#init--create-policy-files)
  - [`run` — mount, execute, unmount](#run--mount-execute-unmount)
  - [`explain` — debug visibility](#explain--debug-visibility)
- [Policy Files](#policy-files)
- [Building from Source](#building-from-source)
- [Development](#development)

## Quick Start

**Install via your favorite package manager:**
```bash
# npm
npm install -g --ignore-scripts @xlansoftware/agentignore

# pnpm
pnpm add -g --ignore-scripts @xlansoftware/agentignore

# bun
bun add -g --ignore-scripts @xlansoftware/agentignore

# run it
agentignore run bash
```

**Install via crate:**
```bash
cargo install agentignore
agentignore run bash
```

**Or build from source with Cargo:**

```bash
# 1. Ensure FUSE is available (see Prerequisites below)

# 2. Build the binary
cargo build --release

# 3. Create example policy files in your project
./target/release/agentignore init

# 4. Start a shell inside the filtered view
./target/release/agentignore run bash
```

## Prerequisites for Running

AgentIgnore needs the **FUSE kernel module** and **libfuse3** on the host. Setup differs slightly across environments.

### Linux (bare-metal)

```bash
# Ubuntu / Debian
sudo apt update
sudo apt install fuse3

# Fedora / RHEL
sudo dnf install fuse3

# Arch
sudo pacman -S fuse3

# Load the kernel module (usually auto-loads on access, but this guarantees it)
sudo modprobe fuse

# Verify
ls -l /dev/fuse          # should be present
fusermount3 --version    # should print 3.x
```

No additional configuration needed — run the binary directly.

### Docker / Dev Container

When running inside a container, FUSE support must be explicitly granted at container creation time:

```bash
docker run \
  --device /dev/fuse \
  --cap-add SYS_ADMIN \
  my-image
```

For VS Code Dev Containers, add to `.devcontainer/devcontainer.json`:

```json
{
  "runArgs": [
    "--device", "/dev/fuse",
    "--cap-add", "SYS_ADMIN"
  ]
}
```

> **Why `SYS_ADMIN`?**  The `mount(2)` syscall (which FUSE uses internally) requires `CAP_SYS_ADMIN` inside a container. The `--device /dev/fuse` flag exposes the kernel FUSE interface.

Inside the container, `libfuse3` is usually preinstalled. If not:

```bash
sudo apt update && sudo apt install fuse3
```

### WSL2

WSL2 supports FUSE out of the box with recent kernels:

```bash
# Install fuse3 inside your WSL distribution (Ubuntu example)
sudo apt update
sudo apt install fuse3

# Load the module
sudo modprobe fuse
```

> **Known issue:** Custom WSL2 kernels built before Linux 5.4 may lack FUSE support. Run `uname -r` to check your kernel version. If FUSE is unavailable, update your WSL kernel via `wsl --update` from a Windows PowerShell.

## Usage

### `init` — create policy files

Creates `.agentignore` and `.agentallow` files in a directory:

```bash
agentignore init              # current directory
agentignore init /path/project
```

### `run` — mount, execute, unmount

Mount a filtered view, run a command inside it, and unmount automatically:

```bash
# Run bash inside a filtered view of the current directory
agentignore run bash

# Run a specific tool with arguments
agentignore run -- ls -la

# Run from a specific source directory
agentignore run --source /home/user/project bash 
```

### `explain` — debug visibility

Check whether a path would be hidden and why:

```bash
agentignore explain .env
# HIDDEN
# canonical path: "/project/.env"
# matched rule:   .env

agentignore explain src/main.rs
# VISIBLE
# canonical path: "/project/src/main.rs"
```

## Policy Files

### `.agentignore`

Uses standard [gitignore](https://git-scm.com/docs/gitignore) pattern syntax. Files matching these patterns are hidden from the agent.

```gitignore
# secrets
.env
*.pem
*.key

# hidden directories
secrets/
node_modules/

# recursive patterns
**/*.tfstate

# negations (exceptions)
!important.env
```

### `.agentallow`

Process-based bypass rules. Processes matching entries in this file can **see hidden files** — useful for development tools that need full access.

Example use case: The agent should not "see" files within `node_modules` but if it launch `npm run build`, the tool needs these files to build the app.

One entry per line. Empty lines and lines starting with `#` are ignored (comments).

#### Process name / cmdline matching

| Syntax | Behaviour |
|--------|-----------|
| `^npm run` | Regex matched against the process `comm` name or its full cmdline; also checks ancestor processes (parent, grandparent, etc.). |
| `=npm` | Exact match against `comm` or cmdline; ancestors are also checked. |
| `=bash!` | Exact match, no ancestor walk — matches only the exact process. |
| `node!` | Regex, no ancestor walk — matches only the exact process. |

#### Binary path matching (entry starts with `/`)

| Syntax | Behaviour |
|--------|-----------|
| `/usr/bin/node` | Literal path; matches process or any ancestor. |
| `/usr/bin/java!` | Literal path; matches only that exact process. |

```
# Allow npm and its children to see everything (regex, walks ancestors)
npm

# Allow only the exact 'restic backup' command (exact match, no ancestor walk)
=restic backup!

# Allow the exact bash process, not commands run inside it (exact, no ancestor walk)
=bash!

# Allow any java process or its children (regex, walks ancestors)
java

# Allow a specific binary and everything under it (literal path, walks ancestors)
/usr/bin/git

# Allow only that exact binary path (literal path, no ancestor walk)
/usr/bin/git!
```

## Building from Source

### Build prerequisites

See [`https://rust-lang.org/tools/install/`](https://rust-lang.org/tools/install/) how to prepare rust dev environmet or use the provided devcontainer in this repo.

### Build commands

```bash
# Fast type-check during development
cargo check

# Debug build
cargo build

# Optimized release build
cargo build --release

# Run with cargo
cargo run -- mount /real/project /mnt/agent

# Run tests
cargo test

# Run linter and formatter before committing
cargo clippy -- -D warnings
cargo fmt
```

## Development

Or you can create one yourself. See [`docs/project.md`](docs/project.md) for a one-shot prompt to re-create the project.
