# PRD: Live Dashboard for `agentignore mount`

## Problem Statement

Running `agentignore mount <source> <mountpoint>` currently blocks forever with no feedback except an initial "Mounting..." line. The user must hit Ctrl+C to see any indication that the mount is still alive. There is no way to monitor which files are being accessed, which processes are interacting with the mount, how the filesystem is performing, or whether policy denials are occurring. This makes it hard to debug policy rules, observe agent behaviour, or gain confidence that the mount is working correctly.

## Solution

The `agentignore mount` command shall display a live terminal dashboard while the filesystem is mounted, showing:

- Real-time operation throughput (ops/second per FUSE operation type)
- Cumulative operation totals since mount
- The last 10 unique files accessed, with per-path hit counters, process name, and colour-coded access status (allowed / denied / bypassed)
- Current number of open file handles
- Mount source and mountpoint paths

The dashboard refreshes every 500ms and exits cleanly on Ctrl+C or external unmount.

## User Stories

1. As an agentignore user, I want to see how many FUSE operations per second are happening, so that I can gauge filesystem activity at a glance.
2. As an agentignore user, I want to see cumulative totals for each operation type (lookup, getattr, read, write, open, etc.), so that I can understand long-term access patterns.
3. As an agentignore user, I want to see the last 10 unique files that were accessed, so that I can identify which paths the agent is touching.
4. As an agentignore user, I want each recent path entry to show how many times it has been accessed, so that I can spot hot paths.
5. As an agentignore user, I want each recent path entry to show the process name that accessed it, so that I can correlate activity with the calling process.
6. As an agentignore user, I want allowed accesses shown in green, denied (hidden) accesses shown in red, and bypassed (`.agentallow`-allowed) accesses shown in yellow, so that I can immediately spot policy hits.
7. As an agentignore user, I want to see the number of currently open file handles, so that I can detect leaked handles or long-lived file sessions.
8. As an agentignore user, I want the dashboard to refresh at a configurable interval (default 500ms), so that I can trade off refresh smoothness against CPU usage.
9. As an agentignore user, I want the dashboard to return to the shell cleanly when I press Ctrl+C, without leaving the terminal in a broken state.
10. As an agentignore user, I want the dashboard to detect external unmounts (e.g. `fusermount -u`) and exit gracefully, so that unmounting from another terminal works as expected.

## Implementation Decisions

### Dependencies

No new external dependencies. The dashboard uses standard ANSI escape codes for colour and terminal control, and the existing `ctrlc` crate for signal handling. The `std::io::stdout()` API is used for output flushing.

### Design Principle: Stats-Only-When-Mounted

Stats collection is **only active when the mount command is running with the dashboard enabled**. When `agentignore mount` is called without the dashboard (e.g. `--no-dashboard` flag), or when `agentignore run` or any other command uses `AgentFS`, the filesystem operates with zero stats overhead — no atomic counters, no mutex acquisitions, no memory allocated for path tracking.

This is achieved by making the stats collector **optional** on `AgentFS`. Every FUSE trait method checks a simple `Option<Arc<StatsCollector>>` before recording anything — when `None`, the call is a no-op and compiles to a single branch.

## Module: `src/fs/stats.rs` (NEW)

A deep module encapsulating the stats collector. Public surface:

```rust
pub struct StatsCollector { /* internals */ }

impl StatsCollector {
    pub fn new() -> Arc<Self>;
    pub fn record_op(&self, op: OpType, path: &Path, pid: u32, kind: AccessKind);
    pub fn record_handle_open(&self);
    pub fn record_handle_close(&self);
    pub fn snapshot(&self) -> Snapshot;
}

pub struct Snapshot {
    pub ops: BTreeMap<OpType, (u64, u64)>,  // (total, tick_count)
    pub recent_paths: Vec<PathSnapshot>,
    pub open_handles: usize,
    pub uptime: Duration,
    pub source: PathBuf,
    pub mountpoint: PathBuf,
}

pub struct PathSnapshot {
    pub path: PathBuf,
    pub pid: u32,
    pub process_name: String,
    pub access: AccessKind,
    pub hit_count: usize,
    pub last_op: OpType,
}

pub enum OpType {
    Lookup, Getattr, Readdir, Open, Read, Write, Release,
    Readlink, Create, Mkdir, Unlink, Rmdir, Rename, Link, Symlink, Statfs,
}

pub enum AccessKind {
    Allowed,    // visible, not matched by policy
    Denied,     // hidden by .agentignore, returned ENOENT
    Bypassed,   // hidden by policy but allowed by .agentallow
}
```

Key behaviours:
- `record_op` increments both the permanent counter and a tick counter (reset each snapshot) for the given `OpType`.
- `recent_paths` maintains up to 10 unique paths. A path's entry is updated (hit_count++, access kind, pid, timestamp) on each new access rather than inserting a duplicate. The list is ordered by most-recently-touched first.
- Paths are deduplicated by full canonical path string. No LRU eviction — only new unique paths push older entries off the bottom.
- All counters use `AtomicU64`; `recent_paths` uses a `Mutex<VecDeque<PathEntry>>`.

### Module: `src/fs.rs` (MODIFY)

- `AgentFS` gains a `stats: Option<Arc<StatsCollector>>` field.
- `AgentFS::new(root)` becomes `AgentFS::new(root, stats: Option<Arc<StatsCollector>>)`.
- Every FUSE trait method (`lookup`, `getattr`, `readdir`, `open`, `read`, `write`, `release`, `readlink`, `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `link`, `symlink`, `statfs`) calls `self.stats.record_op(...)` at its entry point with the current path, pid, and `AccessKind::Allowed`.
- `is_hidden_for_request` is the single choke point for denials. When it returns `true`, a denial event is recorded with `AccessKind::Denied` (or `AccessKind::Bypassed` if the request PID matched `.agentallow` but the path was still hidden from non-bypass processes — though this case is actually an "allowed" bypass, not a denial; see AccessKind semantics above).
- `HandleTable::insert` calls `stats.record_handle_open()`. `HandleTable::remove` calls `stats.record_handle_close()`.

### CLI Flag: `--no-dashboard`

A new optional flag `--no-dashboard` / `-D` is added to `agentignore mount`. When provided:

- No `StatsCollector` is created.
- `AgentFS` is constructed with `stats: None`.
- The dashboard loop is skipped entirely — the mount thread is simply joined after Ctrl+C.
- Output is minimal (the original "Mounting..." line, then "Unmounted.").

```console
$ agentignore mount /project /mnt/agent
# (dashboard shown)

$ agentignore mount --no-dashboard /project /mnt/agent
Mounting...
^C
Unmounted.
```

### Module: `src/cmd/mount.rs` (MODIFY)

The `mount()` function is restructured:

```text
mount(source, mountpoint, no_dashboard):
    canonicalize source
    create mountpoint if needed

    stats = if no_dashboard { None } else { Some(Arc::new(StatsCollector::new())) }
    create AgentFS(root, stats)
    create shutdown_flag (Arc<AtomicBool>)

    set Ctrl+C handler → sets shutdown_flag

    spawn thread:
        fuser::mount2(fs, mountpoint, config)
        // returns when unmounted externally

    if no_dashboard:
        // just wait for shutdown, no rendering
        loop:
            sleep(100)
            if shutdown_flag.load(): break
            if FUSE thread has finished: break
    else:
        // dashboard loop
        print initial dashboard
        loop:
            snapshot = stats.snapshot()
            render dashboard to stdout
            flush
            sleep(REFRESH_MS)
            if shutdown_flag.load(): break
            if FUSE thread has finished: break

    // shutdown
    if not externally unmounted:
        fusermount -u mountpoint
    join FUSE thread
    if we created mountpoint:
        remove_dir(mountpoint)
    print "Unmounted." to stdout
```

The dashboard loop uses ANSI escape sequences:
- `\x1b[2J\x1b[H` — clear screen and home cursor (full redraw each tick, simpler than incremental)
- `\x1b[1;31m` etc. — standard ANSI 8-colour set (red, green, yellow, cyan, reset)
- No alternate screen switching — the dashboard replaces the terminal content naturally

The `REFRESH_MS` constant is defined at module scope:

```rust
const REFRESH_MS: u64 = 500;
```

### Layout (ASCII wireframe)

```
┌─ agentignore mount ─────────────────── uptime: 00:12:37 ───────────┐
│                                                                  │
│  OPS/SEC              TOTAL OPS                                 │
│  LOOKUP      47 ████  14,231                                    │
│  GETATTR     23 ██     6,789                                    │
│  READDIR      3 ▏       912                                     │
│  OPEN        12 █      3,450                                     │
│  READ        89 ████████ 28,102                                  │
│  WRITE        5 ▏      1,023                                     │
│  RELEASE     12 █      3,445                                     │
│  CREATE       1         17                                       │
│  MKDIR        0         0                                        │
│  UNLINK       0         5                                        │
│  RMDIR        0         0                                        │
│  RENAME       0         2                                        │
│  LINK         0         0                                        │
│  SYMLINK      0         0                                        │
│  READLINK     0         0                                        │
│  STATFS       0         1                                        │
│  DENIED       2 ▏      204                                       │
│                                                                  │
│  ──── LAST ACCESSED FILES ───────────── Open handles: 3 ────── │
│  1. R  src/main.rs           proc/pi      ████  47 read(s)      │
│  2. W  /var/log/output.log   proc/bash    ██     3 write(s)     │
│  3. R  .env                  proc/ls      ██ DENIED             │
│  4. R  /secrets/api.key      proc/cat     ▄▄ BYPASS (allow)    │
│  5. R  README.md             proc/ls      █     1 read(s)       │
│  6. R  src/lib.rs            proc/ls      ██    4 read(s)       │
│  7. R  Cargo.toml            proc/ls      ██    3 read(s)       │
│  8.   (idle)                                                      │
│  9.   (idle)                                                      │
│ 10.   (idle)                                                      │
│                                                                  │
│  Mounted: /project → /mnt/agent    Ctrl+C to unmount             │
└──────────────────────────────────────────────────────────────────┘
```

Bar graph width is proportional to the tick ops count relative to the maximum across all operation types.

### Colour scheme

| Element | ANSI Colour |
|---------|-------------|
| Allowed path entry | Green (`\x1b[32m`) |
| Denied path entry | Red (`\x1b[31m`) |
| Bypassed path entry | Yellow (`\x1b[33m`) |
| Header / borders | Cyan (`\x1b[36m`) |
| Numeric values | Default (white) |
| Labels | Default |

## Testing Decisions

### What makes a good test

- Tests exercise external behaviour through the public API, not internal structure
- Stats tests: verify that recording operations produces correct snapshots by calling `record_op` and asserting on `snapshot()`
- Integration tests: mount a temp directory, perform filesystem operations through FUSE, and verify stats collection works end-to-end

### Modules to test

- **`src/fs/stats.rs`** — unit tests for the StatsCollector in isolation. Prior art: `src/fs/policy.rs` has no tests written yet, so this is fresh territory. Focus on:
  - Recording a single op vs multiple ops produces correct totals and tick counts
  - Snapshot resets tick counters
  - Recent paths maintains 10 unique entries with hit_count increments
  - Recent paths drops oldest entry when 11th unique path is recorded
  - Concurrency: multiple threads recording ops don't lose counts
  - Open handles count increments and decrements correctly

- **`src/cmd/mount.rs`** — hard to unit test due to FUSE/threading. Test via integration tests in `tests/`. Prior art: `tests/fs.rs`, `tests/agentallow.rs`, `tests/ignore.rs`. Focus on:
  - Dashboard starts and displays correct source/mountpoint paths
  - Ctrl+C unmounts cleanly
  - External unmount is detected and dashboard exits

### What to skip

- No tests for the exact ANSI formatting output (brittle)
- No tests for bar graph width calculation (minor rendering detail)
- No latency benchmarks for stats collection (overhead is negligible)

## Out of Scope

- A full TUI framework (ratatui, crossterm, etc.) — pure ANSI is sufficient
- The `agentignore run` command — it will continue to use its existing ephemeral mount flow without the dashboard and without a StatsCollector
- Persistent logging of stats to disk
- Network-accessible stats endpoint (HTTP/Unix socket)
- Configurable which operation types to display
- Per-process breakdown of stats
- Database/file-backed history beyond the last-10 recent paths
- Mouse interaction in the dashboard
- Stats collection for non-mount commands (`agentignore run`, `agentignore ls`, etc.) — these pass `None` for the stats collector

## Further Notes

- The `REFRESH_MS` constant is deliberately a `const u64` at the top of `src/cmd/mount.rs` for discoverability. Future work could expose it as a CLI flag.
- The `AccessKind::Bypassed` variant covers the case where a path is matched by `.agentignore` but the requesting process is on the `.agentallow` list. This is technically an "allowed" access, but colouring it differently from both "allowed" and "denied" lets the user see that `.agentallow` is working.
- The existing `setup_signal_handler` in `common.rs` calls `std::process::exit(1)` — the new mount flow replaces this with an `AtomicBool`-based graceful shutdown. The function may still be used by `agentignore run` and should remain unchanged.
- The dashboard renders to stdout. All tracing/log output (via `tracing`) continues to stderr and is not captured or suppressed by the dashboard.
