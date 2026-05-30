use procfs::process::Process;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub fn get_process_name(pid: u32) -> Option<String> {
    Process::new(pid as i32)
        .ok()
        .and_then(|p| p.stat().ok())
        .map(|stat| stat.comm)
}

// ── ProcessNameCache ────────────────────────────────────────────────────────

/// Per-PID process name cache with a configurable TTL.
///
/// Prevents redundant `/proc/<pid>/stat` reads when the same PID touches
/// many files in quick succession (e.g., `find`, `make`).
struct ProcessNameCache {
    map: HashMap<u32, (String, Instant)>,
}

impl ProcessNameCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    fn get_or_load(&mut self, pid: u32) -> Option<String> {
        let now = Instant::now();

        // Evict stale entries
        self.map.retain(|_, &mut (_, expires)| expires > now);

        if let Some((name, _)) = self.map.get(&pid) {
            return Some(name.clone());
        }

        let name = get_process_name(pid)?;
        self.map
            .insert(pid, (name.clone(), now + Duration::from_secs(5)));
        Some(name)
    }
}

static PROCESS_NAME_CACHE: std::sync::LazyLock<Mutex<ProcessNameCache>> =
    std::sync::LazyLock::new(|| Mutex::new(ProcessNameCache::new()));

/// Return the process name for `pid`, using a short-lived cache.
///
/// Cache TTL is 5 seconds.  PIDs that exit are evicted on the next access.
pub fn get_process_name_cached(pid: u32) -> Option<String> {
    PROCESS_NAME_CACHE.lock().unwrap().get_or_load(pid)
}
