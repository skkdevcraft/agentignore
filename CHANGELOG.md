# Changelog

## [0.4.3] — 2026-06-03

### Added

- **Integration tests for `agentignore run` cleanup** — new `tests/run.rs`
  suite verifies that mountpoint directories are cleaned up after both
  valid and invalid commands, and that error messages appear on stderr.

### Changed

- **`agentignore run` error handling** — command execution failures (e.g.,
  binary not found) are now handled gracefully: the mountpoint is always
  unmounted and cleaned up before the process exits, and a descriptive
  error message is printed to stderr. Previously the `status()` call would
  panic on failure, skipping cleanup entirely.
- **`create_temp_mountpoint` naming** — when the simple mountpoint name
  already exists, the fallback now includes a random hex suffix derived
  from a nanosecond timestamp mixed with the PID, guaranteeing uniqueness
  even under concurrent test or process execution.

### Fixed

- **Cleanup on bad `run` commands** — `agentignore run` with a
  nonexistent binary previously skipped unmount and directory removal
  (the mountpoint leaked). Now cleanup runs unconditionally.

## [0.4.2] — 2026-06-02

### Added

- **npm distribution** — agentignore is now published on npm as
  `@xlansoftware/agentignore`. Install with `npm install -g --ignore-scripts
  @xlansoftware/agentignore`, or via pnpm/bun. Cross-compiled for
  `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.
- **`npm-build` make target** for cross-compiling and generating the npm
  package via `cargo npm`.
- **Dangling symlink visibility** — symlinks whose targets do not exist are
  now visible through the mount (previously rejected with `ENOENT`).
- **`resolve_child` helper** — internal refactor splitting `lookup_child` into
  a lower-level `resolve_child` that returns the access path and resolved
  target separately, enabling better symlink-aware security checks.

### Changed

- **Symlink inode tracking** — inodes are now assigned to the symlink's
  *access path* (the symlink entry itself), not the canonicalized target.
  Symlinks and their targets receive distinct inodes. Two symlinks pointing
  to the same target also get distinct inodes.
- **`lookup_child` return type** — expanded from `(PathBuf, u64)` to
  `(PathBuf, Option<PathBuf>, u64)` where the second element is the resolved
  canonical target (`None` for dangling symlinks).
- **`unlink` and `rmdir`** — operate on the access path rather than the
  canonical target, preventing accidental deletion of symlink targets when
  removing the symlink itself.
- **`stat` on symlinks** — returns correct symlink metadata (size = length
  of target path string, not target file content size).
- **Security checks on rename** — destination path is now verified against
  both escape and hidden-file rules on the resolved target.
- **`stat` test fixture** — `test_dir` helper no longer applies
  `atime`/`mtime` (the old approach was racy and unnecessary for symlink
  tests).

### Fixed

- **Hidden-target symlink bypass** — symlinks pointing to hidden files
  (e.g., `.env`) are now correctly rejected at lookup time. Previously
  the canonicalization resolved through the symlink, but the hidden check
  was inconsistent with the new access-path inode model.
- **Symlink deletion semantics** — `unlink` on a symlink no longer removes
  the target file.

## [0.3.0] — 2026-06-01

### Added

- **`doctor` subcommand** — checks that all FUSE prerequisites are met before
  mounting: verifies `fusermount3` is installed, `/dev/fuse` is accessible,
  `libfuse3` is present, and the `fuse` kernel module is loaded.
- **`--show-dashboard` / `-d` flag** (replaces `--no-dashboard`). The dashboard
  is now hidden by default; pass this flag to enable the live stats view.
  The default access summary is printed on mount regardless.

### Changed

- **Flag inversion**: `--no-dashboard` removed in favour of `--show-dashboard`.
  The dashboard is now opt-in, making the default mount output cleaner for
  scripting and headless use.
- **Dashboard output refactored** — internal code optimised for fewer
  allocations and clearer structure.

## [0.2.0] — 2026-06-01

### Added

- **`--show-config-files` / `-c` flag** on `mount` and `run` commands — makes
  `.agentignore` and `.agentallow` files visible in the filtered view so users
  can inspect or edit them through the mount.
- **Passthrough FUSE operations** for broader compatibility with tools and
  workflows:
  - `setattr` — supports chmod, chown, truncate, and utimens (atime/mtime)
    by delegating to the real filesystem.
  - `fsync` / `fdatasync` — syncs the underlying file descriptor.
  - `flush` — no-op (the real kernel-managed fd handles close-time flushing).
  - `fsyncdir` — no-op (stateless directory I/O).
  - `access` — checks real filesystem permissions after policy filtering.
- **Stats tracking** for all new passthrough operations (`Setattr`, `Flush`,
  `Fsync`, `Fsyncdir`, `Access`) in the live dashboard and `check` output.

## [0.1.0] — 2025-05-30

### Added

- Initial release of agentignore — a FUSE filesystem that hides files matching
  `.agentignore` rules from processes, giving developers fine-grained control
  over what AI coding agents can see.
- Core FUSE operations: `lookup`, `getattr`, `readdir`, `open`, `read`, `write`,
  `release`, `readlink`, `rename`, `link`, `symlink`, `create`, `mkdir`, `unlink`,
  `rmdir`, `statfs`.
- `.agentignore` support with per-directory cascading rules (gitignore-based).
- `.agentallow` support for process-based bypass rules with regex/exact matching,
  PID ancestor walks, and binary path matching.
- Config hot-reload with a fast atomic-timestamp check path.
- Inode table with cache eviction on policy reload and rename.
- Handle table for open file tracking.
- Stats collector for deny/bypass/operation tracking.
- Comprehensive test suite with 88+ tests.
