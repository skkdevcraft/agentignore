# Changelog

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
