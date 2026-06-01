//! `agentignore mount` — Mount a filtered view of a source directory at a mountpoint.
//!
//! When `--no-dashboard` is not given, displays a live terminal dashboard that
//! refreshes every 500ms showing operation throughput, cumulative totals,
//! recently accessed files, and open handle counts.

use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use agentignore::fs::AgentFS;
use agentignore::fs::stats::Snapshot;
use agentignore::fs::stats::{AccessKind, OpType, StatsCollector};

/// Dashboard refresh interval in milliseconds.
const REFRESH_MS: u64 = 500;

/// Handle `agentignore mount <source> <mountpoint>`.
pub fn mount(source: PathBuf, mountpoint: PathBuf, show_dashboard: bool, show_config_files: bool) {
    // Verify FUSE prerequisites before attempting to mount
    crate::cmd::doctor::check_prerequisites(true);

    let source = source.canonicalize().expect("source path must exist");

    // Create mountpoint if it doesn't exist
    let was_created = if !mountpoint.exists() {
        std::fs::create_dir_all(&mountpoint).expect("failed to create mountpoint");
        true
    } else {
        false
    };

    // Always create the stats collector so we can track recent paths
    // even in no-dashboard mode
    let stats = {
        let s = StatsCollector::new();
        s.set_source(source.clone());
        s.set_mountpoint(mountpoint.clone());
        Some(s)
    };

    let fs = AgentFS::with_config(source.clone(), stats.clone(), show_config_files);

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

    if !show_dashboard {
        // ── No-dashboard mode ──────────────────────────────────────────────
        // Print newly accessed paths since the last check, one per line.
        println!("Mounting AgentIgnore: {:?} → {:?}", source, mountpoint);

        let stats = stats.expect("stats should be Some in no-dashboard mode");

        loop {
            std::thread::sleep(std::time::Duration::from_millis(REFRESH_MS));

            let snap = stats.snapshot(true);

            // Print paths that appeared since the last check
            for ps in snap.recent_paths.iter() {
                let colour = access_colour(ps.access);
                let op_letter = op_letter(ps.last_op);
                let suffix = format_hit_suffix(ps.hit_count, ps.access, ps.last_op);
                let display_path = format_path_display(&ps.path, 40);
                let proc_name = format_truncated_name(&ps.process_name, 16);

                println!("{colour}{op_letter} {suffix} {proc_name:<14} {display_path}{ANSI_RESET}");
            }

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
        println!("Mounting AgentIgnore: {:?} → {:?}", source, mountpoint);
        println!("Dashboard active — Ctrl+C to unmount\n");

        let stats = stats.expect("stats should be Some in dashboard mode");

        loop {
            let snap = stats.snapshot(false);

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

/// Render the full dashboard to stdout using ANSI escape codes.
fn render_dashboard(snap: &Snapshot) {
    let mut out = String::new();

    // Clear screen and home cursor
    out.push_str("\x1b[2J\x1b[H");

    // ── Header ─────────────────────────────────────────────────────────────
    let uptime = format_uptime(snap.uptime);
    let _ = writeln!(
        out,
        "{ANSI_CYAN}┌─ agentignore mount ─────────────── uptime: {uptime} ───────────┐{ANSI_RESET}"
    );

    // ── OPS table ──────────────────────────────────────────────────────────
    let _ = writeln!(
        out,
        "  {ANSI_CYAN}OPS/SEC              TOTAL OPS{ANSI_RESET}"
    );

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
            "  {label:<10} {display_tick} {ANSI_CYAN}{bar}{ANSI_RESET}  {display_total}"
        );
    }

    // ── Recent files ──────────────────────────────────────────────────────
    let _ = writeln!(
        out,
        "  {ANSI_CYAN}──── LAST ACCESSED FILES {ANSI_RESET}───────────── {ANSI_CYAN}Open handles: {}{ANSI_RESET} ──────",
        snap.open_handles,
    );

    if snap.recent_paths.is_empty() {
        for i in 1..=10 {
            let _ = writeln!(out, "{i:>2}.  {ANSI_GRAY}(idle){ANSI_RESET}");
        }
    } else {
        for (i, ps) in snap.recent_paths.iter().enumerate() {
            let num = i + 1;
            let colour = access_colour(ps.access);
            let op_letter = op_letter(ps.last_op);
            let display_path = format_path_display(&ps.path, 40);
            let proc_name = format_truncated_name(&ps.process_name, 16);
            let suffix = format_hit_suffix(ps.hit_count, ps.access, ps.last_op);

            // Hit bar (8 chars proportional to hit_count, relative to max hits)
            let max_hits = snap
                .recent_paths
                .iter()
                .map(|p| p.hit_count)
                .max()
                .unwrap_or(1)
                .max(1);
            let hit_bar = bar_chart(ps.hit_count as u64, max_hits as u64, 8);

            let _ = writeln!(
                out,
                "{num:>2}. {colour}{op_letter}  {display_path}  {proc_name:<14} {hit_bar}  {suffix}{ANSI_RESET}"
            );
        }
    }

    // ── Footer ─────────────────────────────────────────────────────────────
    let _ = writeln!(
        out,
        "  {ANSI_CYAN}Mounted: {:?} → {:?}    Ctrl+C to unmount{ANSI_RESET}",
        snap.source, snap.mountpoint,
    );
    let _ = writeln!(
        out,
        "{ANSI_CYAN}└──────────────────────────────────────────────────────────────────┘{ANSI_RESET}"
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

// ── ANSI colour constants ───────────────────────────────────────────────────

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_GRAY: &str = "\x1b[90m";

// ── Rendering helpers ───────────────────────────────────────────────────────

/// Return the ANSI colour code for an access kind.
fn access_colour(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Allowed => ANSI_GREEN,
        AccessKind::Denied => ANSI_RED,
        AccessKind::Bypassed => ANSI_YELLOW,
    }
}

/// Return the trailing label for an access kind (including leading space).
fn access_suffix(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Allowed => "",
        AccessKind::Denied => " DENIED",
        AccessKind::Bypassed => " BYPASS",
    }
}

/// Single-letter abbreviation for an operation type.
fn op_letter(op: OpType) -> &'static str {
    match op {
        OpType::Read => "R",
        OpType::Write => "W",
        _ => "A",
    }
}

/// Format a path for display: left-padded to `width`, with ellipsis
/// truncation when the path exceeds `width`.
fn format_path_display(path: &std::path::Path, width: usize) -> String {
    let s = path.display().to_string();
    if s.len() > width {
        let (_, tail) = s.split_at(s.len().saturating_sub(width - 3));
        format!("...{tail}")
    } else {
        format!("{s:<width$}")
    }
}

/// Truncate a process name to at most `max_chars` characters.
fn format_truncated_name(name: &str, max_chars: usize) -> String {
    if name.len() > max_chars {
        name[..max_chars.saturating_sub(3)].to_string()
    } else {
        name.to_string()
    }
}

/// Build a proportional unicode bar chart string (█ characters).
fn bar_chart(value: u64, max: u64, width: u32) -> String {
    let w = if max > 0 {
        ((value as f64 / max as f64) * width as f64).round() as usize
    } else {
        0
    };
    "\u{2588}".repeat(w)
}

/// Format the hit-count suffix for a path snapshot.
fn format_hit_suffix(hit_count: usize, access: AccessKind, last_op: OpType) -> String {
    let label = access_suffix(access);
    if matches!(access, AccessKind::Denied | AccessKind::Bypassed) {
        format!("{hit_count}{label}")
    } else if last_op == OpType::Write {
        format!("{hit_count} W")
    } else {
        format!("{hit_count} R")
    }
}
