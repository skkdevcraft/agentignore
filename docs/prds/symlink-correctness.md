# PRD: Symlink Correctness — Proper Inode Tracking, Metadata, and Mutation

## Problem Statement

As an agentignore user, symlinks inside the mount behave in surprising and dangerous ways. Calling `unlink` on a symlink removes the **target file** instead of the symlink itself. `readlink` fails with `ENOENT`. The symlink and its target share the same inode number, so `stat` on a symlink returns the target's attributes (regular file, not symlink). Renaming a symlink silently renames the target. Directory listings report a `FileType::Symlink` entry, but any subsequent operation on that inode operates on the target. These are not cosmetic issues — they cause data loss and violate core POSIX semantics.

## Solution

Fix symlink handling at the inode-assignment layer so that the inode table stores the **access path** (the path the kernel asked about) rather than the **canonical resolved target**. Security checks (escape detection, hidden-target validation) still follow symlinks to their ultimate target — but the inode number, metadata, and mutation operations (unlink, rmdir, rename) operate on the symlink node itself, not the resolved target.

## User Stories

1. As an AI agent working inside an agentignore mount, I want to `unlink` a symlink and have the symlink removed (not the target file), so that my scripts and build pipelines don't silently destroy real data.

2. As an AI agent, I want `readlink /path/to/link` to return the symlink target, so that symlink resolution works as expected and tools that traverse symlinks (like `realpath`, shell completion, recursive `find -L`) behave correctly.

3. As an AI agent, I want `ls -l` to show `lrwxrwxrwx` for a symlink and report the correct symlink size and target, so that the directory listing accurately reflects the filesystem structure.

4. As an AI agent, I want symlinks and their targets to have distinct inode numbers, so that tools like `find -samefile`, `stat`, and hard-link detection don't conflate a symlink with its target.

5. As an AI agent, I want symlinks pointing outside the mount root to be denied when resolved, but still report the correct `FileType::Symlink` if the link itself is inside root, so that escape detection works without breaking metadata.

6. As an AI agent, I want symlinks pointing to hidden targets to be denied with `ENOENT`, so that hidden files remain inaccessible through symlink traversal.

7. As an AI agent, I want `rename` on a symlink to rename the symlink node (not the target), so that file reorganization doesn't silently affect unrelated files.

8. As an agentignore user relying on `.agentallow` bypass rules, I want process-based bypasses to still work when accessing symlink targets, so that trusted processes can access hidden data through symlinks if explicitly permitted.

9. As an agentignore maintainer, I want the symlink-correctness fix to be implemented at the `lookup_child` layer so that all FUSE operations (lookup, getattr, open, readlink, unlink, rmdir, rename) get the corrected behavior from a single change point.

10. As an agentignore user, I want symlinks inside the mount to behave exactly as they do on the backing filesystem (modulo policy hiding), so that the mount is a true passthrough for visible paths.

11. As an AI agent, I want creating symlinks inside the mount to work with POSIX semantics (`ln -s target link`), with symlink targets validated against escape and hidden-target policy, so that workflows that create symlinks (e.g., `npm link`, `pip install -e`) function correctly.

## Implementation Decisions

### Core Fix: Separate `lookup_child` into Two Phases

`lookup_child` currently calls `std::fs::canonicalize()` and uses the result as both the inode-table key and the path for downstream operations. This is the root cause of every bug.

**Decision:** Split `lookup_child` into two path concepts:

1. **Access path** (`child_real`): The logical path formed by joining the parent's stored path with the child name. This is what gets stored in the inode table. It is NOT canonicalized — it may be a symlink, and that's correct.

2. **Security target** (`resolved`): The path after following all symlinks (via `canonicalize` or manual symlink-chain resolution). This is used ONLY for escape checks and hidden-target validation. It is NEVER stored in the inode table.

Pseudocode of the new behavior:

```rust
// lookup_child returns (access_path, resolved_for_security, inode)
pub fn lookup_child(&self, parent: INodeNo, name: &OsStr, req: Option<&Request>)
    -> Option<(PathBuf, Option<PathBuf>, u64)>
{
    let parent_real = self.real_path(parent)?;
    let child_real = parent_real.join(name);

    // Resolve symlinks for security checks only
    let resolved = std::fs::canonicalize(&child_real).ok();

    // Escape check on the resolved target
    if let Some(ref r) = resolved {
        if !r.starts_with(&self.root) {
            warn!("DENY path-escape: {child_real:?} → {r:?}");
            return None;
        }
    }

    // Hidden-target check on the resolved target
    if let Some(ref r) = resolved {
        if self.is_hidden_for_request(r, req) {
            debug!("DENY lookup hidden via symlink: {child_real:?} → {r:?}");
            return None;
        }
    }

    // Inode stored at the access path (NOT canonicalized)
    let ino = self.guard.inodes_write().get_or_insert(&child_real);
    Some((child_real, resolved, ino))
}
```

### Inode Table: Paths Are Now Access Paths, Not Canonical Paths

The inode table previously stored canonical paths because `lookup_child` and `readdir` both called `canonicalize()` before inserting. After this fix, the inode table stores the joined logical path. Two symlinks pointing to the same target get distinct inodes. The symlink and its target get distinct inodes. This matches POSIX semantics.

### `stat()` Called with the Access Path

`stat()` calls `std::fs::symlink_metadata(real)` where `real` is now the access path. If the access path is a symlink, `symlink_metadata` correctly reports `FileType::Symlink` with the symlink's size and mode. Previously, `real` was the canonical target path, so `symlink_metadata` returned the target's attributes.

### `readlink` Repaired

`readlink` gets the path from `real_path(ino)`, which after the fix returns the access path (the symlink itself). `std::fs::read_link` is called on that path and succeeds. The target resolution for security checks (escape, hidden) is done separately on the resolved target.

### `unlink` / `rmdir` / `rename` Operate on the Symlink Node

These FUSE trait methods construct `child_real` by joining the parent's stored access path with the name, then call `std::fs::canonicalize()` to get a **resolved** path for the hidden/escape checks only. The actual mutation (`remove_file`, `remove_dir`, `rename`) is performed on `child_real` — the logical path, which may be a symlink. This means:

- `unlink(link)` removes the symlink, not the target. Target survives.
- `rmdir(dir-link)` calls `remove_dir` on the symlink path. Since `remove_dir` resolves symlinks (per POSIX), this still removes the target directory. This is POSIX-correct: `rmdir` on a symlink to a directory does remove the target directory. However, the security check ("is the symlink's target hidden?") prevents removal of hidden targets.
- `rename(link, newname)` renames the symlink, not the target.

### `readdir` Inode Consistency

`readdir` currently has an inconsistency: it reports `FileType::Symlink` (from `entry.file_type()`, which is `lstat`-based) but stores the canonicalized path in the inode table (which follows symlinks). After the fix, `readdir` stores the joined child path (not canonicalized) in the inode table, matching the reported type. The `is_hidden_for_request` check is performed on the canonicalized/resolved path for security.

### `symlink` (creation) Validation Preserved

The existing security check in `symlink` — resolving the target and checking for escapes and hidden paths — remains unchanged. The inode for the newly created symlink is stored using the access path (`link_real`), not the resolved target.

### `lookup` FUSE Handler Adjusted

The `lookup` handler calls `stat(ino, &access_path)` to get attributes. Since `access_path` is now the symlink path, `stat` correctly reports symlink attributes. The `getattr` handler similarly uses `real_path(ino)` which now returns the access path.

### Symlink Chains

Multiple levels of symlink indirection (A → B → C) are handled naturally: each `lookup_child` resolves the full chain for security checks but stores only the intermediate access path. The kernel drives walk through each component sequentially via repeated `lookup` calls. No special handling needed.

### `open` / `read` / `write` Still Follow Symlinks

`open` uses the path from `real_path(ino)`, which is now the access path. When the kernel calls `open` on a symlink, `std::fs::OpenOptions::open()` follows the symlink naturally (this is OS behavior, not our code). This is correct POSIX semantics: opening a symlink opens the target. We do not need special logic here.

### Dangling Symlinks

A symlink whose target does not exist is still visible in directory listings. `canonicalize` fails, so `resolved` is `None`. The security checks are skipped (no target to check). The inode is assigned to the symlink access path. `open` will fail with the backing filesystem's error. This is POSIX-correct.

### Refactoring: Extract Path Resolution Helper

To keep `unlink`, `rmdir`, `rename`, `readlink`, and `link` DRY, extract a helper on `AgentFS`:

```rust
/// Resolve a (parent_ino, name) pair into:
/// - access_path: the literal path (may be a symlink)
/// - resolved: the canonicalized target for security checks (None if doesn't exist)
///
/// Returns None if the resolved target escapes root or is hidden.
fn resolve_child(
    &self,
    parent: INodeNo,
    name: &OsStr,
    req: Option<&Request>,
) -> Option<(PathBuf, Option<PathBuf>)>
```

All FUSE handlers that need to resolve `(parent, name)` pairs call this helper, which enforces the access-path-vs-security-target split consistently.

### Backward Compatibility

This change breaks no existing public API. `lookup_child` gains a new return field; callers are updated. `AgentFS` struct public fields and methods (`root`, `stat`, `is_hidden`, `is_hidden_for_request`, `real_path`, `readlink`) remain signature-compatible. The `readlink` FUSE handler behavior changes from broken (always `ENOENT`) to working, which is a fix, not a regression.

## Testing Decisions

### What Makes a Good Test

Tests must only verify **externally observable behavior** of `AgentFS` — not internal state like the inode table's HashMap contents. Good test: "After `lookup_child` on a symlink, `stat` reports `FileType::Symlink`." Bad test: "The inode table's `path_to_ino` map contains the access path."

### Testing Seams (Highest to Lowest)

**Seam 1: `AgentFS::lookup_child()`** — Tests call `lookup_child` with `None` for the request parameter (existing pattern in `tests/agentfs.rs`). Verify returned path is the access path, not the canonical target. Test escape rejection, hidden-target rejection, and dangling symlink acceptance.

**Seam 2: `AgentFS::stat()`** — Tests call `stat` with a symlink path. Verify `kind == FileType::Symlink` and `size` matches the symlink length, not the target file size. Already partially tested (`agentfs_stat_for_symlink`).

**Seam 3: `AgentFS::is_hidden_for_request()`** — Tests create a symlink to a hidden target, verify the check returns `true` (hidden) for the resolved target. Creates a symlink to a visible target, verify `false`.

**Seam 4: `AgentFS::resolve_child()`** (new helper) — Tests create symlinks with various target scenarios (inside root, outside root, hidden, deleted). Verifies the helper correctly splits access path from resolved target and enforces security checks.

**Seam 5: FUSE handler-level tests** — For `readlink`, `unlink`, `rmdir`, `rename`: the FUSE trait methods require `&Request` which is hard to construct. Instead, test the `resolve_child` helper exhaustively, then add targeted end-to-end tests that verify file system state before/after mutations (create symlink, call helper with temp dir, verify via `std::fs` operations that symlink is handled, not target).

### Prior Art

- `tests/agentfs.rs` already tests `lookup_child` with symlinks (`agentfs_lookup_child_path_escape`, `agentfs_lookup_child_symlink_to_file_inside_root`). These will be updated to match corrected behavior.
- `tests/agentfs.rs` already tests `stat` with symlinks (`agentfs_stat_for_symlink`). This test remains unchanged — it already asserts `FileType::Symlink`.
- `tests/common/mod.rs` provides `test_dir()`, `touch()`, `mkdirp()` helpers. Symlink creation via `std::os::unix::fs::symlink` is already used in existing tests.

### Tests to Add

1. `lookup_child` on symlink returns access path ending in link name (not target name)
2. `lookup_child` on symlink to outside root returns `None`
3. `lookup_child` on symlink to hidden target returns `None`
4. `lookup_child` on dangling symlink returns `Some`
5. `stat` on symlink path reports `FileType::Symlink` with correct size
6. `is_hidden_for_request` returns `true` for symlink to hidden target
7. `resolve_child` escape check: symlink to `/etc/passwd` → `None`
8. `resolve_child` hidden check: symlink to `.env` → `None`
9. Symlinks to same target get distinct inodes from `lookup_child`
10. Symlink inode is distinct from target inode
11. `readlink` via FUSE handler (integration-level, if feasible)
12. `lookup_child` + manual `remove_file` on access path → target survives (indirect unlink test)

## Out of Scope

- Symlink-to-symlink chains longer than OS limits (handled by `canonicalize` naturally)
- Cyclic symlink detection (handled by `canonicalize` returning `ELOOP`)
- Symlinks whose targets reside on different filesystems (cross-device handling)
- Performance optimization of the canonicalization path (canonicalize is a syscall cost; this PRD addresses correctness, not speed)
- Symlink behavior under FUSE kernel caching / attribute timeout interactions
- Mount-level integration tests requiring `fusermount` and kernel module support
- Changing how `open()` follows symlinks (this is OS-level behavior and correct per POSIX)

## Further Notes

- The existing test `agentfs_lookup_child_symlink_to_file_inside_root` asserts `path.ends_with("realfile.txt")` — this is the current broken behavior. It will be updated to assert `path.ends_with("link.txt")` (the access path), with a sibling assertion that the security check still passes because the resolved target is inside root and visible.
- The `resolve_child` helper is additive — it does not change the interface of `lookup_child` beyond adding the resolved-target return value. Callers that don't need the resolved target can ignore it.
- The `symlink` (create) FUSE handler already stores the access path (`link_real`) in the inode table, not the resolved target. This is already correct and unchanged.
- `readdir`'s escape fast-reject (`child_real.starts_with(&self.root)`) was added in a previous PRD and remains correct — it operates on the joined access path before canonicalization.
