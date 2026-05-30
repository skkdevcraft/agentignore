# PRD: Performance Hardening — Lock Contention, Caching, and Hot-Path Optimisation

## Problem Statement

As an agentignore user, mounts become effectively single-threaded under concurrent FUSE traffic. A `make -j16` inside the mount or a parallel `find` crawler stalls because **every filesystem operation acquires the policy write-lock**, even when no configuration files have changed on disk. Additionally, redundant filesystem syscalls (re-canonicalizing already-canonical paths, re-reading `/proc/<pid>/stat` for every new path entry) and unnecessary `Mutex` contention in read-heavy data structures waste CPU and amplify latency. The filesystem is functionally correct but architecturally hostile to concurrency and high-throughput workloads.

## Solution

A targeted performance pass over the FUSE call chain that eliminates serialisation bottlenecks, removes redundant syscalls, and improves data-structure lock granularity. The mount remains a transparent passthrough with zero correctness regressions — every hidden-path denial, symlink escape block, and `.agentallow` bypass continues to work identically. The changes are purely mechanical: split read/write lock paths, reorder checks to fail fast, cache stable values, and swap `Mutex` for `RwLock` where readers dominate.

## User Stories

1. As an agentignore user running a build inside the mount (`make -j16`), I want concurrent FUSE operations to proceed in parallel without serialising on the policy lock, so that build throughput matches the backing filesystem.

2. As an agentignore user running a parallel crawler (`find . -type f | xargs -P 8 cat`), I want lookups and getattrs to be served concurrently, so that crawler wall-clock time is not dominated by lock contention.

3. As an agentignore user, I want `ensure_policy_fresh()` to be a near-zero-cost operation when no config files have changed, so that 99.999% of FUSE ops skip the policy reload hot path entirely.

4. As an agentignore user, I want the dashboard (when enabled) to collect stats without adding a `canonicalize()` syscall on top of the one already performed in the FUSE operation, so that stats overhead is minimized.

5. As an agentignore user, I want the `readdir` path to fast-reject children that fall outside the mount root *before* calling `canonicalize()`, so that directories with many entries don't incur per-child syscall costs unnecessarily.

6. As an agentignore user, I want process-name lookups for the recent-paths dashboard to be cached so that the same PID touching many files in quick succession doesn't re-read `/proc/<pid>/stat` each time.

7. As an agentignore user, I want the inode table to serve concurrent read-only lookups without blocking each other, so that parallel `stat` or `read` calls don't contend on the inode lock.

8. As an agentignore user, I want the policy matcher cache to serve concurrent `is_hidden()` evaluations without writers blocking readers, so that the common-case hidden-file check is fully parallel.

9. As an agentignore user, I want the stats collector's recent-paths update to avoid unnecessary cloning when a path is re-hit, so that the dashboard hot path is allocation-efficient.

10. As an agentignore maintainer, I want a dedicated `PolicyFreshnessGuard` module that encapsulates the read-check-then-write-reload split, so that the reload protocol is testable in isolation and the FUSE trait implementation stays clean.

## Implementation Decisions

### Lock Granularity Split: Policy Write vs. Read

The `AgentFS::ensure_policy_fresh()` method currently acquires `self.policy.write().unwrap()` unconditionally. This serialises all concurrent FUSE operations on a single exclusive lock, even though 99.999% of calls find no config changes.

**Decision:** Split the policy freshness check into two phases:

1. **Fast path (under `RwLock::read`)**: Compare a cached `last_check` timestamp (stored as `AtomicU64` seconds-since-epoch on `AgentFS`) against wall-clock time. Skip entirely if the last check was <1 second ago. If sufficient time has passed, call a new `Policy::has_config_changed() -> bool` method that reads `config_mtimes` without mutating state. If nothing changed, update the timestamp and return.

2. **Slow path (under `RwLock::write`)**: Only acquire the write lock when `has_config_changed()` returns `true`. Call the existing `check_and_reload()`, evict inode table entries, and clear the matcher cache.

This converts the common case from an exclusive write-lock acquisition to a read-lock acquisition (or a no-op timestamp check), allowing true concurrency.

### New Deep Module: `PolicyFreshnessGuard`

A standalone wrapper that isolates the read-check/write-reload protocol. Interface:

- Constructed with references to `&RwLock<Policy>`, `&RwLock<InodeTable>`, and `root: &Path`.
- `check(&self) -> (bool, u64)` — returns `(reloaded: bool, checked_at: u64)`. Internally gates on `last_check` atomic, calls `has_config_changed` under read lock, and only escalates to write when needed.

`AgentFS` owns one `PolicyFreshnessGuard` and calls it at the top of each FUSE method. The guard returns immediately on the fast path, making the per-op overhead a single `AtomicU64::load` plus a timestamp comparison.

### `Policy::has_config_changed()`

A new read-only method on `Policy` that compares each entry in `config_mtimes` against the current on-disk mtime (via `std::fs::metadata().modified()`). No mutation, no allocation. Returns `true` if any file's mtime has changed since `last_loaded`.

This is the counterpart to `check_and_reload()`. If `has_config_changed()` returns false, no write lock is needed. If it returns true, `check_and_reload()` takes the write lock and performs the full reload.

### `InodeTable` Behind `RwLock`

Change `AgentIgnore::inodes` from `Mutex<InodeTable>` to `RwLock<InodeTable>`. The `path()` method is read-heavy (every `getattr`, `read`, `write`, `lookup`); only `get_or_insert`, `evict_prefix`, and `parent_ino` need write access. `readdir` becomes a mixed case: `parent_ino` is read, but `get_or_insert` for each visible child is write.

**Decision:** Keep individual `get_or_insert` calls under write lock in `readdir`. The cost of upgrading a read lock to write is higher than just grabbing write. For mutating operations (`create`, `mkdir`, `link`, `symlink`), the write lock is held only around `get_or_insert`, not the entire method body.

### `Policy::matcher_cache` Behind `RwLock`

Change from `Mutex<HashMap<PathBuf, PathBuf>>` to `RwLock<HashMap<PathBuf, PathBuf>>`. Reads dominate: every `is_hidden()` call reads the cache. Writes happen on cache miss (first access to a path) or on policy reload (full clear).

### `StatsCollector` Canonicalization Removal

The `record_op` method currently calls `path.canonicalize()` on every invocation. Every FUSE call site already passes a canonical path. Remove the `canonicalize()` call and trust callers. The `lookup` callback is the one exception — it calls `record_op` with `Path::new(name)`, a bare filename. Fix that call site to pass the canonical child path instead (already computed by `lookup_child`).

```text
// Before:
record_op(OpType::Lookup, path, req.pid(), ...);  // path is bare filename
// After:
record_op(OpType::Lookup, &real, req.pid(), ...);  // real is canonical from lookup_child
```

### `readdir` Fast-Reject Before Canonicalize

The `readdir` loop currently canonicalizes every child unconditionally. Since the parent directory path is already canonical (from `lookup_child` or `real_path`), the joined child path is safe to check against `self.root` before canonicalizing:

```text
for entry in entries {
    let child_real = entry.path();
    // FAST REJECT — pure string comparison, no syscall:
    if !child_real.starts_with(&self.root) {
        warn!("DENY path-escape in readdir: {child_real:?}");
        continue;
    }
    // Now canonicalize only for real checks:
    let canonical = std::fs::canonicalize(&child_real).ok()?;
    // ... rest of policy checks
}
```

### Process-Name Cache for Stats Dashboard

The dashboard hot path (`StatsCollector::record_op`) calls `tools::get_process_name(pid)` which reads `/proc/<pid>/stat` via `procfs`. For a PID that touches many files (e.g., `find`), this is N redundant reads.

**Decision:** Introduce a simple `ProcessNameCache` in `tools.rs` with per-PID caching and a configurable TTL. Use `HashMap<u32, (String, Instant)>` behind a `Mutex`. The existing `allow_list.rs` `ProcessCache` is wider (comm, cmdline, exe, ppid) and a different shape — keep them separate for now to avoid coupling the stats path to the allow-list module.

Interface:
```text
pub fn get_process_name_cached(pid: u32) -> Option<&'static str>
```
with an internal `lazy_static` or `OnceCell` cache. PIDs that exit are evicted on next access.

### Stats Collector VecDeque Optimisation

When a path already exists in the `recent` VecDeque, the code currently clones the entry, then searches for and removes the old position, then pushes the clone to the front. Instead, take ownership via `remove()` directly and mutate in place, avoiding one allocation:

```text
if let Some(idx) = recent.iter().position(|e| e.path == canonical) {
    let mut entry = recent.remove(idx).unwrap(); // take ownership
    entry.hit_count += 1;
    entry.access = kind;
    entry.pid = pid;
    entry.last_op = op;
    recent.push_front(entry);
}
```

### Two-Lock Elimination in `is_hidden_for_request`

Currently many FUSE methods acquire the policy lock twice: once for `ensure_policy_fresh()` and again for `is_hidden_for_request()`. After the `ensure_policy_fresh` fast-path change, both are read-lock acquisitions, but still two atomic operations. Combine them by inlining the freshness timestamp check into `is_hidden_for_request`, reducing to a single lock hold:

```text
pub fn is_hidden_for_request(&self, path: &Path, req: Option<&Request>) -> bool {
    self.ensure_policy_fresh(); // fast-path: timestamp check, maybe read lock
    let policy = self.policy.read().unwrap();
    // ...rest of logic...
}
```

This stays as two calls but the `ensure_policy_fresh` is now nearly always a no-op. Further merging risks tangling concerns — the two-call pattern is acceptable when the first call is cheap.

## Testing Decisions

The user has marked testing as **out of scope** for this PRD.

## Out of Scope

- Switching to a lock-free concurrent hashmap (e.g., `dashmap`, `evmap`)
- Replacing `std::sync::RwLock` with `parking_lot::RwLock` for fairness/performance
- Adding an LRU eviction policy to the inode table (currently unbounded growth)
- Adding directory-level result caching for `readdir` (stale-read risk)
- Changing the TTL constant from `Duration::from_secs(1)` to a tunable value
- Benchmarks (`criterion`) — the project has no existing benchmark infrastructure
- Profiling with `perf` or `flamegraph` beyond the code analysis already done
- Test coverage for the performance changes

## Further Notes

- The `PolicyFreshnessGuard` module is designed as a drop-in wrapper — it can be swapped back to the old `ensure_policy_fresh()` in one place if needed for debugging.
- The readdir fast-reject optimization assumes that `child_real` (a join of a canonical parent with a non-`..` name) is a valid path. This holds because FUSE guarantees child names from the kernel don't contain `/` or `..`.
- The process-name cache should use a bounded TTL (e.g., 5 seconds) rather than staying keyed forever, since PIDs are reused by the kernel.