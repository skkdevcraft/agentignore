# Changelog

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

## [0.1.0] — 2025-06-30

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
