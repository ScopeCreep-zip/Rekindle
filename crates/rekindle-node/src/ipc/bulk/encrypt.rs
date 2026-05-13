//! Dedicated rayon thread pool for CPU-bound bulk encryption/decryption.
//!
//! Constructed once at process startup, shared via `Arc` across all
//! connections and bulk transfer sessions.
//!
//! Workers are named for debuggability (visible in `ps -T`, `top -H`,
//! `perf top`). The pool is separate from the global rayon pool and
//! from Tokio's blocking pool to prevent thread oversubscription.
//!
//! # Why rayon and not `tokio::task::spawn_blocking`
//!
//! `spawn_blocking` targets blocking I/O (file reads, DNS lookups)
//! with an unbounded thread pool that grows on demand. CPU-bound AES-GCM
//! work on that pool competes with actual blocking I/O and causes thread
//! oversubscription. Rayon's work-stealing scheduler is purpose-built
//! for CPU-bound parallelism with a fixed thread count.

use std::sync::Arc;
use rayon::ThreadPool;

/// Detect physical core IDs by parsing sysfs topology.
///
/// Returns CPU IDs that are the first thread of each physical core.
/// On i5-9300H: typically `[0, 2, 4, 6]` (HT siblings are 1, 3, 5, 7).
/// Falls back to sequential IDs if topology is unavailable.
#[cfg(target_os = "linux")]
fn detect_physical_cores() -> Vec<usize> {
    let mut physical_cores = Vec::new();
    let mut seen_cores = std::collections::HashSet::new();

    for cpu_id in 0..1024 {
        let path = format!(
            "/sys/devices/system/cpu/cpu{cpu_id}/topology/thread_siblings_list"
        );
        let Ok(content) = std::fs::read_to_string(&path) else { break };
        let first_sibling = content
            .trim()
            .split([',', '-'])
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(cpu_id);

        if seen_cores.insert(first_sibling) {
            physical_cores.push(first_sibling);
        }
    }

    if physical_cores.is_empty() {
        let n = std::thread::available_parallelism()
            .map(|n| n.get().min(4))
            .unwrap_or(2);
        (0..n).collect()
    } else {
        physical_cores
    }
}

#[cfg(not(target_os = "linux"))]
fn detect_physical_cores() -> Vec<usize> {
    let n = std::thread::available_parallelism()
        .map(|n| n.get().min(4))
        .unwrap_or(2);
    (0..n).collect()
}

/// Cached physical core IDs. Computed once, reused by encrypt_workers()
/// and build_encrypt_pool()'s start_handler.
pub fn cached_physical_cores() -> &'static Vec<usize> {
    use std::sync::OnceLock;
    static CORES: OnceLock<Vec<usize>> = OnceLock::new();
    CORES.get_or_init(detect_physical_cores)
}

/// Number of encryption worker threads.
///
/// Reserves 2 physical cores for the Tokio I/O runtime and uses the
/// remainder for encryption, capped at 4. On a 4-core machine this
/// means 2 encrypt workers; on 8-core, 4 workers.
///
/// Override with `REKINDLE_ENCRYPT_WORKERS=N` for explicit control.
fn encrypt_workers() -> usize {
    if let Ok(val) = std::env::var("REKINDLE_ENCRYPT_WORKERS") {
        if let Ok(n) = val.parse::<usize>() {
            if n >= 1 {
                return n.min(cached_physical_cores().len());
            }
        }
    }
    cached_physical_cores().len().saturating_sub(2).clamp(1, 4)
}

/// Build the dedicated encryption thread pool.
///
/// Call once at daemon startup. Share the returned `Arc` across all
/// connection handlers and bulk transfer sessions.
///
/// Workers are pinned to detected physical cores (not HT siblings)
/// to avoid AES-NI port 0/5 contention between hyperthreads.
pub fn build_encrypt_pool() -> Arc<ThreadPool> {
    let workers = encrypt_workers();
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .thread_name(|i| format!("rekindle-encrypt-{i}"))
            .start_handler({
                let cores = cached_physical_cores().clone();
                move |idx| {
                    #[cfg(target_os = "linux")]
                    {
                        if let Some(&target_cpu) = cores.get(idx) {
                            // SAFETY: zeroed cpu_set_t is valid (all-zeros = no CPUs).
                            // CPU_SET sets one bit. sched_setaffinity with pid=0
                            // targets current thread. Returns -1 on failure (non-fatal).
                            unsafe {
                                let mut set = std::mem::zeroed::<libc::cpu_set_t>();
                                libc::CPU_SET(target_cpu, &mut set);
                                let result = libc::sched_setaffinity(
                                    0,
                                    std::mem::size_of::<libc::cpu_set_t>(),
                                    &raw const set,
                                );
                                if result == 0 {
                                    tracing::debug!(
                                        worker = idx, cpu = target_cpu,
                                        "encrypt worker pinned to physical core"
                                    );
                                }
                            }
                        }
                    }
                    let _ = idx;
                }
            })
            .build()
            .expect("failed to build encryption thread pool"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn pool_has_correct_thread_count() {
        let pool = build_encrypt_pool();
        assert_eq!(pool.current_num_threads(), encrypt_workers());
    }

    #[test]
    fn pool_executes_work() {
        let pool = build_encrypt_pool();
        let counter = Arc::new(AtomicUsize::new(0));
        let n = 100;

        pool.scope(|s| {
            for _ in 0..n {
                let c = Arc::clone(&counter);
                s.spawn(move |_| {
                    c.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        assert_eq!(counter.load(Ordering::Relaxed), n);
    }

    #[test]
    fn detect_physical_cores_returns_nonempty() {
        let cores = detect_physical_cores();
        assert!(!cores.is_empty(), "must detect at least one physical core");
    }
}
