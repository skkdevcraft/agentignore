//! `agentfs mount` — Mount a filtered view of a source directory at a mountpoint.
//!
//! When `--no-dashboard` is not given, displays a live terminal dashboard that
//! refreshes every 500ms showing operation throughput, cumulative totals,
//! recently accessed files, and open handle counts.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use agentfs::fs::AgentFS;
use agentfs::fs::stats::{AccessKind, OpType, StatsCollector};

/// Dashboard refresh interval in milliseconds.
const REFRESH_MS: u64 = 500;

/// Handle `agentfs mount <source> <mountpoint>`.
pub fn mount(source: PathBuf, mountpoint: PathBuf, no_dashboard: bool) {
    let source = source.canonicalize().expect("source path must exist");

    // Create mountpoint if it doesn't exist
    let was_created = if !mountpoint.exists() {
        std::fs::create_dir_all(&mountpoint).expect("failed to create mountpoint");
        true
    } else {
        false
    };

    // Create stats collector (or not)
    let stats = if no_dashboard {
        None
    } else {
        let s = StatsCollector::new();
        s.set_source(source.clone());
        s.set_mountpoint(mountpoint.clone());
        Some(s)
    };

    let fs = AgentFS::with_stats(source.clone(), stats.clone());

    // Shutdown flag for graceful Ctrl+C handling
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_signal = shutdown.clone();
    let mp_unmount = mountpoint.clone();

    ctrlc::set_handler(move || {
        shutdown_signal.store(true, Ordering::SeqCst);
        eprintln!("\nReceived interrupt. Unmounting {:?}...", mp_unmount);

        // Attempt unmount via fusermount
        let status = std::process::Command::new("fusermount")
            .args(["-u", mp_unmount.to_str().unwrap()])
            .status();
        if let Ok(status) = status
            && !status.success()
        {
            eprintln!("Warning: fusermount failed for {:?}", mp_unmount);
        }
    })
    .expect("failed to set Ctrl+C handler");

    // Spawn the FUSE mount in a background thread
    let mountpoint_clone = mountpoint.clone();
    let mount_handle = std::thread::Builder::new()
        .name("fuse-mount".into())
        .spawn(move || {
            fuser::mount2(fs, &mountpoint_clone, &fuser::Config::default()).expect("mount failed");
        })
        .expect("failed to spawn mount thread");

    if no_dashboard {
        // ── No-dashboard mode ──────────────────────────────────────────────
        println!("Mounting AgentFS: {:?} → {:?}", source, mountpoint);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if shutdown.load(Ordering::SeqCst) {
                break;
            }
            if mount_handle.is_finished() {
                // External unmount
                break;
            }
        }
    } else {
        // ── Dashboard mode ────────────────────────────────────────────────
        println!("Mounting AgentFS: {:?} → {:?}", source, mountpoint);
        println!("Dashboard active — Ctrl+C to unmount\n");

        let stats = stats.expect("stats should be Some in dashboard mode");

        loop {
            let snap = stats.snapshot();

            render_dashboard(&snap);

            // Check for shutdown or external unmount
            if shutdown.load(Ordering::SeqCst) {
                break;
            }
            if mount_handle.is_finished() {
                // FUSE thread exited (external unmount)
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(REFRESH_MS));
        }
    }

    // ── Cleanup ────────────────────────────────────────────────────────────

    // If the mount thread is still running, unmount externally so it exits
    if !mount_handle.is_finished() {
        let status = std::process::Command::new("fusermount")
            .args(["-u", mountpoint.to_str().unwrap()])
            .status();
        if let Ok(status) = status
            && !status.success()
        {
            eprintln!("Warning: fusermount failed for {:?}", mountpoint);
        }
    }

    // Wait for mount thread to finish
    let _ = mount_handle.join();

    // Remove mountpoint if we created it
    if was_created {
        let _ = std::fs::remove_dir(&mountpoint);
    }

    println!("Unmounted.");
}

// ── Dashboard rendering ─────────────────────────────────────────────────────

use agentfs::fs::stats::Snapshot;
use std::fmt::Write;

/// Render the full dashboard to stdout using ANSI escape codes.
fn render_dashboard(snap: &Snapshot) {
    let mut out = String::new();

    // Clear screen and home cursor
    out.push_str("\x1b[2J\x1b[H");

    // ── Header ─────────────────────────────────────────────────────────────
    let uptime = format_uptime(snap.uptime);
    let _ = writeln!(
        out,
        "\x1b[36m┌─ agentfs mount ─────────────────── uptime: {uptime} ───────────┐\x1b[0m"
    );

    // ── OPS table ──────────────────────────────────────────────────────────
    let _ = writeln!(out, "  \x1b[36mOPS/SEC              TOTAL OPS\x1b[0m");

    // Find max tick count for bar scaling
    let max_tick = snap.ops.values().map(|&(_, t)| t).max().unwrap_or(1).max(1);

    for op_type in OpType::ALL {
        let (total, tick) = snap.ops.get(op_type).copied().unwrap_or((0, 0));
        let label = op_type.label();

        // Bar graph: 8 chars wide, proportional to max_tick
        let bar_width = if max_tick > 0 {
            ((tick as f64 / max_tick as f64) * 8.0).round() as usize
        } else {
            0
        };
        let bar = "\u{2588}".repeat(bar_width); // █

        // Format tick count (right-aligned to 4 chars)
        let display_tick = format!("{:>4}", tick);
        let display_total = format_thousands(total);

        let _ = writeln!(
            out,
            "  {label:<10} {display_tick} \x1b[36m{bar}\x1b[0m  {display_total}"
        );
    }

    // ── Recent files ──────────────────────────────────────────────────────
    let _ = writeln!(
        out,
        "  \x1b[36m──── LAST ACCESSED FILES \x1b[0m───────────── \x1b[36mOpen handles: {}\x1b[0m ──────",
        snap.open_handles,
    );

    if snap.recent_paths.is_empty() {
        for i in 1..=10 {
            let _ = writeln!(out, "{i:>2}.  \x1b[90m(idle)\x1b[0m");
        }
    } else {
        for (i, ps) in snap.recent_paths.iter().enumerate() {
            let num = i + 1;
            let path_str = ps.path.display().to_string();
            // Trim long paths
            let display_path = if path_str.len() > 40 {
                let (_, tail) = path_str.split_at(path_str.len().saturating_sub(37));
                format!("...{tail}")
            } else {
                format!("{:<40}", path_str)
            };

            // Colour based on access kind
            let (colour, access_label) = match ps.access {
                AccessKind::Allowed => ("\x1b[32m", ""),
                AccessKind::Denied => ("\x1b[31m", " DENIED"),
                AccessKind::Bypassed => ("\x1b[33m", " BYPASS"),
            };

            let op_letter = match ps.last_op {
                OpType::Read => "R",
                OpType::Write => "W",
                _ => "A",
            };

            let proc_name = if ps.process_name.len() > 16 {
                &ps.process_name[..13]
            } else {
                &ps.process_name
            };

            // Hit bar (8 chars proportional to hit_count, relative to max hits)
            let max_hits = snap
                .recent_paths
                .iter()
                .map(|p| p.hit_count)
                .max()
                .unwrap_or(1)
                .max(1);
            let hit_bar_width = ((ps.hit_count as f64 / max_hits as f64) * 8.0).round() as usize;
            let hit_bar = "\u{2588}".repeat(hit_bar_width);

            let suffix = if matches!(ps.access, AccessKind::Denied | AccessKind::Bypassed) {
                format!("{}(s){access_label}", ps.hit_count)
            } else if ps.last_op == OpType::Write {
                format!("{} write(s)", ps.hit_count)
            } else {
                format!("{} read(s)", ps.hit_count)
            };

            let _ = writeln!(
                out,
                "{num:>2}. {colour}{op_letter}  {display_path}  {proc_name:<14} {hit_bar}  {suffix}\x1b[0m"
            );
        }
    }

    // ── Footer ─────────────────────────────────────────────────────────────
    let _ = writeln!(
        out,
        "  \x1b[36mMounted: {:?} → {:?}    Ctrl+C to unmount\x1b[0m",
        snap.source, snap.mountpoint,
    );
    let _ = writeln!(
        out,
        "\x1b[36m└──────────────────────────────────────────────────────────────────┘\x1b[0m"
    );

    // Write to stdout and flush
    use std::io::Write as IoWrite;
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
}

/// Format a `Duration` as `HH:MM:SS`.
fn format_uptime(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// Format a number with thousands separators.
fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}
