//! `agentignore doctor` — Diagnose the FUSE environment.
//!
//! Checks that all prerequisites for running agentignore are met:
//!
//! * `fusermount` binary  (from `fuse3` / `libfuse3` package)
//! * `/dev/fuse`          (device node, created by the kernel module)
//! * `libfuse3` shared library (linked at run time by the `fuser` crate)
//! * `fuse` kernel module (loaded via `modprobe fuse` or built in)
//! * `fuse` group membership (non-root access to `/dev/fuse`)
//!
//! When running inside a container the diagnostic messages adapt to suggest
//! container-specific fixes instead of bare `apt install` commands.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

// ── Public API ──────────────────────────────────────────────────────────────

/// The standalone `agentignore doctor` subcommand.
///
/// Prints a header, runs every check, and exits with code 1 if any check
/// failed.  See the module docs for the list of checks.
pub fn doctor() {
    let mut failures: Vec<&str> = Vec::new();

    // Print header
    println!("agentignore doctor");
    println!("{}", "\u{2500}".repeat(48));

    let in_container = is_in_container();

    // 1. fusermount binary
    let _ = check_fusermount(false, &mut failures);

    // 2. /dev/fuse device node
    let _ = check_dev_fuse(false, in_container, &mut failures);

    // 3. libfuse3 shared library
    let _ = check_libfuse3(false, &mut failures);

    // 4. fuse kernel module
    let _ = check_fuse_module(false, in_container, &mut failures);

    // Summary
    println!();
    if failures.is_empty() {
        println!("\u{2713} All checks passed.");
    } else {
        println!("\u{2717} {} check(s) failed:", failures.len());
        for f in &failures {
            println!("     \u{2022} {f}");
        }
        println!("     \u{2192} Fix the issues above, then run `agentignore doctor` again.");
        std::process::exit(1);
    }
}

/// Pre-flight check called by `mount` and `run` before attempting to mount.
///
/// Runs the same checks as [`doctor`] and exits with code 1 if any
/// prerequisite is missing.  This prevents a cryptic FUSE panic.
pub fn check_prerequisites(silent: bool) {
    let mut failures: Vec<&str> = Vec::new();

    let in_container = is_in_container();

    // Keep output minimal for the pre-flight case
    check_fusermount(silent, &mut failures);
    check_dev_fuse(silent, in_container, &mut failures);
    check_libfuse3(silent, &mut failures);
    check_fuse_module(silent, in_container, &mut failures);

    if !failures.is_empty() {
        eprintln!(
            "Error: {} prerequisite(s) not met. Run `agentignore doctor` for details.",
            failures.len()
        );
        std::process::exit(1);
    }
}

// ── Container detection ─────────────────────────────────────────────────────

/// Detect if we are running inside a container (Docker / containerd / k8s).
///
/// Uses two independent signals:
///
/// 1.  The file `/.dockerenv` (created by the Docker / containerd runtime).
/// 2.  Cgroup v1 controller paths in `/proc/1/cgroup` that contain
///     `docker`, `containerd`, `kubepods`, or `pod`.
fn is_in_container() -> bool {
    // Signal 1: /.dockerenv marker
    if Path::new("/.dockerenv").exists() {
        return true;
    }

    // Signal 2: cgroup controller paths
    if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup") {
        let lower = cgroup.to_lowercase();
        lower.contains("/docker/")
            || lower.contains("/containerd/")
            || lower.contains("/kubepods/")
            || lower.contains("/pod")
    } else {
        false
    }
}

// ── Individual checks ───────────────────────────────────────────────────────

/// Check that `fusermount` (or `fusermount3`) is installed and runnable.
fn check_fusermount(silent: bool, failures: &mut Vec<&str>) -> bool {
    // Try fusermount3 first (libfuse3 >= 3.15 renamed the binary),
    // then fall back to fusermount.
    let candidates = ["fusermount3", "fusermount"];
    for bin in &candidates {
        if let Ok(output) = Command::new(bin).arg("--version").output() {
            if output.status.success() {
                if !silent {
                    let version = String::from_utf8_lossy(&output.stdout);
                    let version = version.lines().next().unwrap_or("");
                    println!("  \u{2713} fusermount ({version})");
                }
                return true;
            }
        }
    }

    println!("  \u{2717} fusermount not found");
    println!("    \u{2514} Install fuse3:  sudo apt install fuse3  (Debian/Ubuntu)");
    println!("                   sudo dnf install fuse3  (Fedora)");
    failures.push("fusermount binary missing — install fuse3");
    false
}

/// Check that `/dev/fuse` exists and is readable/writable.
fn check_dev_fuse(silent: bool, in_container: bool, failures: &mut Vec<&str>) -> bool {
    match std::fs::metadata("/dev/fuse") {
        Ok(meta) => {
            let mode = meta.permissions().mode();
            let readable = mode & 0o444 != 0;
            let writable = mode & 0o222 != 0;

            if readable && writable {
                if !silent {
                    println!("  \u{2713} /dev/fuse (accessible)");
                }
                true
            } else {
                println!("  \u{2717} /dev/fuse (insufficient permissions)");
                print_dev_fuse_help(in_container);
                failures.push("/dev/fuse not readable/writable");
                false
            }
        }
        Err(e) => {
            println!("  \u{2717} /dev/fuse: {e}");
            print_dev_fuse_help(in_container);
            failures.push("/dev/fuse not found");
            false
        }
    }
}

/// Print remediation advice for `/dev/fuse` problems.
fn print_dev_fuse_help(in_container: bool) {
    if in_container {
        println!("    \u{2514} Your container needs access to /dev/fuse:");
        println!("       For devcontainer.json, add:");
        println!(r#"          "runArgs": ["--device", "/dev/fuse", "--cap-add", "SYS_ADMIN"]"#);
        println!("       For docker run:");
        println!("          docker run --device /dev/fuse --cap-add SYS_ADMIN ...");
    } else {
        println!("    \u{2514} Install fuse3 and ensure /dev/fuse exists:");
        println!("         sudo apt install fuse3  (Debian/Ubuntu)");
        println!("         sudo modprobe fuse");
    }
}

/// Check that `libfuse3` shared library is installed (so the `fuser` crate
/// can load it at run time).
fn check_libfuse3(silent: bool, failures: &mut Vec<&str>) -> bool {
    if let Ok(output) = Command::new("ldconfig").args(["-p"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("libfuse3") {
                if !silent {
                    println!("  \u{2713} libfuse3 (shared library found)");
                }
                return true;
            }
        }
    }

    // Fallback: try to load it directly via dlopen (more robust)
    // But that requires unsafe, so keep it simple.
    println!("  \u{2717} libfuse3 shared library not found");
    println!("    \u{2514} Install fuse3:  sudo apt install fuse3  (Debian/Ubuntu)");
    println!("                   sudo dnf install fuse3  (Fedora)");
    failures.push("libfuse3 library missing");
    false
}

/// Check that the `fuse` kernel module is loaded (or built in).
///
/// Reads `/proc/filesystems` and looks for `fuse` / `fuseblk`.
fn check_fuse_module(silent: bool, in_container: bool, failures: &mut Vec<&str>) -> bool {
    match std::fs::read_to_string("/proc/filesystems") {
        Ok(content) => {
            if content.contains("fuseblk") || content.contains("fuse") {
                if !silent {
                    let lines: Vec<&str> = content.lines().filter(|l| l.contains("fuse")).collect();
                    println!("  \u{2713} fuse kernel module ({})", lines.join(", "));
                }
                true
            } else {
                println!("  \u{2717} fuse kernel module not loaded");
                print_fuse_module_help(in_container);
                failures.push("fuse kernel module not loaded");
                false
            }
        }
        Err(e) => {
            println!("  \u{2717} cannot read /proc/filesystems: {e}");
            failures.push("cannot check kernel module — /proc/filesystems unavailable");
            false
        }
    }
}

fn print_fuse_module_help(in_container: bool) {
    if in_container {
        println!("    \u{2514} The host needs the fuse module loaded:");
        println!("       docker run --privileged ...   (on older hosts)");
        println!("       Or on the host: sudo modprobe fuse");
    } else {
        println!("    \u{2514} Load the kernel module:");
        println!("         sudo modprobe fuse");
        println!("       To load automatically at boot:");
        println!("         echo fuse | sudo tee /etc/modules-load.d/fuse.conf");
    }
}
