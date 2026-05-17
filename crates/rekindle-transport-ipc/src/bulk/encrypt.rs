#![allow(unsafe_code)]
//! Dedicated rayon thread pool for CPU-bound bulk encryption.
//!
//! Workers named for debuggability (visible in ps -T, perf top).
//! Separate from global rayon and tokio blocking pool to prevent
//! thread oversubscription.
//!
//! Workers pinned to physical cores (not HT siblings) to avoid
//! AES-NI port contention between hyperthreads.

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

/// Cached physical core IDs. Computed once.
pub fn cached_physical_cores() -> &'static Vec<usize> {
    use std::sync::OnceLock;
    static CORES: OnceLock<Vec<usize>> = OnceLock::new();
    CORES.get_or_init(detect_physical_cores)
}

/// Number of encrypt workers.
/// Reserves 2 physical cores for tokio, uses rest capped at 4.
/// Override: REKINDLE_ENCRYPT_WORKERS env or explicit count.
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
    // Reserve cores for tokio + OS. On small machines (<=4 cores),
    // use only 2 workers. On larger machines, use cores-2 capped at 4.
    let cores = cached_physical_cores().len();
    if cores <= 4 { 2.min(cores) } else { (cores - 2).min(4) }
}

/// Whether CPU pinning is safe on this machine.
/// Pinning on <=4 cores starves tokio and the OS scheduler.
/// Only pin when there are enough cores that dedicated rayon workers
/// don't contend with the async runtime and interrupt handlers.
fn should_pin_workers() -> bool {
    cached_physical_cores().len() > 4
}

/// Build the dedicated encryption thread pool.
///
/// Workers pinned to detected physical cores. Call once at startup,
/// share the Arc across all connections and bulk sessions.
pub fn build_encrypt_pool(override_workers: usize) -> Arc<ThreadPool> {
    let workers = encrypt_workers(override_workers);
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .thread_name(|i| format!("rekindle-encrypt-{i}"))
            .start_handler({
                let cores = cached_physical_cores().clone();
                let pin = should_pin_workers();
                move |idx| {
                    #[cfg(target_os = "linux")]
                    {
                        // Only pin on machines with >4 cores. On <=4 cores,
                        // pinning starves the tokio runtime and OS scheduler,
                        // causing system-wide lockup under sustained crypto load.
                        if pin {
                            if let Some(&target_cpu) = cores.get(idx) {
                                // SAFETY: CPU_SET + sched_setaffinity with pid=0 targets
                                // current thread. Returns -1 on failure (non-fatal).
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
                // Log the panic. Without a panic_handler, rayon calls process::abort().
                // With this handler, the worker thread exits but the pool continues.
                // The MemoryReservation in the closure is dropped by catch_unwind
                // before this handler is called — no memory leak.
                let msg = if let Some(s) = err.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = err.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic payload".to_string()
                };
                tracing::error!(panic = %msg, "rayon worker panicked — connection should be closed");
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
}
