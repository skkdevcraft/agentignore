# AgentFS — Policy-Filtered Filesystem for AI Agents

## Overview

AgentFS is a Linux userspace filesystem implemented using FUSE.

The filesystem exposes a filtered virtual view of a real directory tree. Files and directories matching rules in a `.agentignore` file are completely hidden from processes interacting with the mounted filesystem.

The primary use case is constraining autonomous AI/LLM agents that have shell access.

The filesystem must make hidden files appear non-existent:

* hidden files must not appear in `ls`
* hidden files must not appear in `find`
* hidden files must not appear in glob expansion
* direct access attempts must return `ENOENT`
* hidden files must not be reachable via symlink traversal
* hidden files must not be discoverable through filesystem metadata

The filesystem should behave as a transparent passthrough filesystem for allowed paths.

---

# Goals

## Primary Goals

* Hide files/directories from AI agents
* Preserve normal filesystem semantics for visible files
* Avoid copying files
* Allow real writes to visible files
* Dynamically evaluate policy rules
* Support gitignore-style patterns
* Prevent path traversal and symlink escapes
* Behave like a normal POSIX filesystem

## Non-Goals

* Full VM/container isolation
* Kernel-level MAC implementation
* Cryptographic protection
* Defense against privileged users
* Antivirus or malware detection

---

# High-Level Architecture

```text
Real filesystem
    ↓
AgentFS (FUSE passthrough filesystem)
    ↓
Mounted filtered view
    ↓
LLM agent / shell tools
```

Example:

```text
/project
    .env
    secrets/
    src/
    README.md
```

`.agentignore`:

```text
.env
secrets/
```

Mounted view:

```text
/mnt/agent
    src/
    README.md
```

The hidden files must appear not to exist.

---

# Technology Stack

## Recommended Language

Rust

## Recommended Libraries

### FUSE

* `fuser` 0.17

### Pattern Matching

One of:

* `ignore`
* `globset`
* custom gitignore-compatible matcher

### Logging

* `tracing`

---

# Filesystem Model

AgentFS is a passthrough filesystem.

The backing filesystem remains authoritative.

AgentFS only:

* mediates visibility
* enforces policy
* filters filesystem operations

No file contents are copied.

All modifications to visible files affect the real filesystem.

---

# Mount Semantics

Example:

```bash
agentfs mount /real/project /mnt/agent
```

Where:

* `/real/project` is the backing root
* `/mnt/agent` is the filtered mountpoint

The mounted filesystem exposes only allowed paths.

---

# Policy File

## Filename

`.agentignore`

## Scope

Applies recursively from mount root.

## Syntax

Gitignore-compatible syntax.

Supported:

* comments
* globs
* recursive patterns
* directory rules
* negation rules

Examples:

```text
# secrets
.env
*.pem
*.key

# hidden directories
secrets/
node_modules/

# recursive
**/*.tfstate

# negate
!important.env
```

---

# Core Security Semantics

## Hidden Paths

Hidden paths must behave as though they do not exist.

Operations against hidden paths must return:

```text
ENOENT
```

NOT:

* `EPERM`
* `EACCESS`
* permission denied

This prevents discoverability.

---

# Filesystem Operations

The following FUSE operations must enforce policy.

---

## readdir

Purpose:

* controls directory listing visibility

Requirements:

* hidden entries must not appear
* filtered before returning results

Affects:

* `ls`
* `find`
* shell completion
* globbing

---

## lookup

Purpose:

* resolves path components

Requirements:

* hidden paths return `ENOENT`

Prevents:

* guessed-path access

---

## getattr

Purpose:

* stat metadata requests

Requirements:

* hidden paths return `ENOENT`

Prevents:

* existence probing

---

## open

Purpose:

* final access enforcement

Requirements:

* hidden files cannot be opened

---

## readlink

Purpose:

* symlink resolution

Requirements:

* symlink targets must be canonicalized
* symlinks escaping into hidden paths must fail

Critical security requirement.

---

## rename

Purpose:

* moving files

Requirements:

* prevent rename-based bypasses

Example attack:

```bash
mv .env allowed.txt
```

Policy must still apply.

---

## link / symlink

Purpose:

* prevent alternate references to hidden files

Requirements:

* hidden inodes cannot become visible via linking

---

## create

Purpose:

* creating new files

Requirements:

* newly created files must immediately participate in policy evaluation

---

# Path Resolution Rules

All paths must be:

1. normalized
2. canonicalized
3. evaluated relative to mount root

Must prevent:

* `..` traversal
* symlink escapes
* bind mount escapes

---

# Policy Engine

## Evaluation Order

1. canonicalize path
2. convert to mount-relative path
3. evaluate against `.agentignore`
4. determine visibility

## Internal Semantics

Prefer allow/deny decision caching.

## Caching

Optional:

* inode cache
* pattern cache
* directory cache

Must support invalidation when:

* `.agentignore` changes
* filesystem changes

---

# Symlink Security

Critical requirement.

The filesystem must prevent:

```text
visible/link -> /real/project/.env
```

Implementation requirements:

* resolve symlink target
* canonicalize final target
* re-run policy evaluation

Hidden targets must fail.

---

# Hard Link Security

Potential attack:

```bash
ln hidden visible
```

Requirements:

* hidden inode must remain inaccessible
* link operations must enforce policy

---

# Error Semantics

Hidden files:

* return `ENOENT`

Invalid permissions:

* return standard POSIX permission errors

Filesystem errors:

* pass through normally

---

# Performance Requirements

## Target Characteristics

* low overhead passthrough
* scalable directory traversal
* minimal metadata amplification

## Optimization Targets

* path cache
* ignore matcher cache
* lazy canonicalization
* batched directory filtering

---

# Logging

Optional structured logging.

Possible events:

* denied access
* hidden path access attempts
* symlink violations
* rename violations

Example:

```text
DENY open /project/.env
DENY symlink escape /project/link
```

---

# Future Features

## Read-only Rules

Example:

```text
readonly:docs/**
```

Behavior:

* readable
* not writable

---

## Virtual Files

Inject synthetic files into mounted view.

Example:

* `/AGENT.md`
* `/POLICY.txt`

---

## Redacted Files

Example:

```text
redact:.env
```

Behavior:

* visible
* contents filtered/redacted

---

## Overlay Mode

Optional writable overlay layer:

* visible writes isolated from backing FS

Similar to overlayfs.

---

## Network Policy

Optional companion sandbox:

* seccomp
* namespace isolation
* network restrictions

---

# Recommended Runtime Architecture

Recommended deployment:

```text
real filesystem
    ↓
AgentFS mount
    ↓
bubblewrap sandbox
    ↓
LLM agent
```

Bubblewrap provides:

* PID isolation
* mount namespace isolation
* proc isolation
* optional network isolation

AgentFS provides:

* filesystem visibility policy

Together they provide strong containment.

---

# CLI Specification

## Mount

```bash
agentfs mount <source> <mountpoint>
```

Example:

```bash
agentfs mount ~/project /mnt/agent
```

---

## Unmount

```bash
agentfs unmount <mountpoint>
```

---

## Validate Policy

```bash
agentfs check
```

Outputs:

* parsed rules
* invalid patterns
* conflicts

---

## Explain Decision

```bash
agentfs explain path/to/file
```

Outputs:

* visible/hidden
* matched rule
* canonical path

Example:

```text
HIDDEN
matched rule: *.pem
canonical path: /project/secrets/key.pem
```

---

# Testing Requirements

## Must Test

### Visibility

* hidden files absent from `ls`
* hidden files absent from `find`

### Direct Access

* hidden files return `ENOENT`

### Symlink Escapes

* blocked

### Rename Attacks

* blocked

### Hard Links

* blocked

### Nested Ignore Rules

* correctly evaluated

### Concurrent Access

* thread-safe behavior

### Large Trees

* acceptable traversal performance

---

# Example Session

## Real Filesystem

```text
/project
    .env
    src/main.rs
    secrets/api.key
```

`.agentignore`:

```text
.env
secrets/
```

Mount:

```bash
agentfs mount /project /mnt/agent
```

Inside mount:

```bash
cd /mnt/agent

ls
```

Output:

```text
src
```

Attempt:

```bash
cat .env
```

Result:

```text
No such file or directory
```

---

# Design Philosophy

The filesystem should implement:

> capability-filtered filesystem visibility for autonomous agents

The policy boundary must exist at the filesystem layer, not:

* prompt layer
* shell wrapper layer
* application layer

The agent must genuinely perceive hidden files as non-existent.
