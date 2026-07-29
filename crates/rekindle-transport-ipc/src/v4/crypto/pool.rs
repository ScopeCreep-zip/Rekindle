//! Dedicated rayon thread pool for CPU-bound bulk encryption.
//!
//! Workers named for debuggability (visible in `ps -T`, `perf top`).
//! Separate from the global rayon pool and tokio's blocking pool to
//! prevent thread oversubscription under sustained crypto load.
//!
//! On machines with >4 physical cores, workers are pinned to physical
//! cores (not HT siblings) to avoid AES-NI port contention between
//! hyperthreads sharing the same execution unit.
//!
//! Build once at server/client startup, share via `Arc<ThreadPool>`
//! across all connections. Multiple clients in the same process share
//! the same pool via the `encrypt_pool` parameter on `IpcClient::connect`.

use std::sync::Arc;
use rayon::ThreadPool;

/// Detect physical core IDs by parsing sysfs topology.
/// Returns first-thread-per-core IDs. Falls back to sequential.
#[cfg(target_os = "linux")]
fn detect_physical_cores() -> Vec<usize> {
    let mut physical_cores = Vec::new();
    let mut seen_cores = std::collections::HashSet::new();

    for cpu_id in 0..1024 {
        let path = format!(
            "/sys/devices/system/cpu/cpu{cpu_id}/topology/thread_siblings_list"
        );
        let Ok(content) = std::fs::read_to_string(&path) else {
            break;
        };
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

/// Cached physical core IDs. Computed once per process.
pub fn cached_physical_cores() -> &'static Vec<usize> {
    use std::sync::OnceLock;
    static CORES: OnceLock<Vec<usize>> = OnceLock::new();
    CORES.get_or_init(detect_physical_cores)
}

/// Compute the number of encrypt workers.
///
/// Default: `physical_cores - 2`, reserving 2 cores for the tokio runtime
/// and OS. Minimum 1 worker on any hardware.
///
/// Override: pass non-zero `override_count` to constrain (e.g., a 92-core
/// server configured to use only 2 workers for a lightweight deployment).
/// Or set `REKINDLE_ENCRYPT_WORKERS` env var.
///
/// Examples:
///   2 cores → 1 worker (minimum)
///   4 cores → 2 workers
///  16 cores → 14 workers
///  92 cores → 90 workers (default), or 2 if override_count=2
fn encrypt_workers(override_count: usize) -> usize {
    if override_count >= 1 {
        return override_count.min(cached_physical_cores().len());
    }
    if let Ok(val) = std::env::var("REKINDLE_ENCRYPT_WORKERS") {
        if let Ok(n) = val.parse::<usize>() {
            if n >= 1 {
                return n.min(cached_physical_cores().len());
            }
        }
    }
    let cores = cached_physical_cores().len();
    (cores.saturating_sub(2)).max(1)
}

/// Whether CPU pinning is safe on this machine.
/// Pinning on ≤4 cores starves tokio and the OS scheduler.
fn should_pin_workers() -> bool {
    cached_physical_cores().len() > 4
}

/// Build the dedicated encryption thread pool.
///
/// Call once at startup. Share the `Arc<ThreadPool>` across all
/// connections and bulk sessions. Workers are pinned to detected
/// physical cores on machines with >4 cores.
///
/// `override_workers`: pass 0 for auto-detect, or a specific count.
#[allow(unsafe_code)]
pub fn build_encrypt_pool(override_workers: usize) -> Arc<ThreadPool> {
    let workers = encrypt_workers(override_workers);
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .stack_size(8 * 1024 * 1024)
            .thread_name(|i| format!("rekindle-v3-encrypt-{i}"))
            .start_handler({
                let cores = cached_physical_cores().clone();
                let pin = should_pin_workers();
                move |idx| {
                    #[cfg(target_os = "linux")]
                    {
                        if pin {
                            if let Some(&target_cpu) = cores.get(idx) {
                                unsafe {
                                    let mut set = std::mem::zeroed::<libc::cpu_set_t>();
                                    libc::CPU_SET(target_cpu, &mut set);
                                    libc::sched_setaffinity(
                                        0,
                                        std::mem::size_of::<libc::cpu_set_t>(),
                                        &raw const set,
                                    );
                                }
                            }
                        }
                    }
                    let _ = idx;
                    let _ = pin;
                }
            })
            .panic_handler(|err| {
                let msg = if let Some(s) = err.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = err.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic payload".to_string()
                };
                tracing::error!(panic = %msg, "v3 encrypt worker panicked");
            })
            .build()
            .expect("failed to build v3 encryption thread pool"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn pool_executes_work() {
        let pool = build_encrypt_pool(0);
        let counter = Arc::new(AtomicUsize::new(0));
        pool.scope(|s| {
            for _ in 0..100 {
                let c = Arc::clone(&counter);
                s.spawn(move |_| {
                    c.fetch_add(1, Ordering::Relaxed);
                });
            }
        });
        assert_eq!(counter.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn detect_cores_nonempty() {
        assert!(!detect_physical_cores().is_empty());
    }

    #[test]
    fn auto_detect_workers_at_least_one() {
        assert!(encrypt_workers(0) >= 1);
    }

    #[test]
    fn override_workers_respected() {
        let n = encrypt_workers(2);
        assert!(n <= 2);
    }
}
