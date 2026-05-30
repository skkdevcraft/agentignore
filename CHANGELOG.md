# Changelog

## [0.1.0] — 2025-06-30

### Added

- Initial release of agentfs — a FUSE filesystem that hides files matching
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
