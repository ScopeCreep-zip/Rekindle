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

/// Number of encryption worker threads.
///
/// Capped at 4 to avoid oversubscribing on typical hardware. On a
/// 2-core machine, uses 2. On 8+ cores, uses 4.
fn encrypt_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().min(4))
        .unwrap_or(2)
}

/// Build the dedicated encryption thread pool.
///
/// Call once at daemon startup. Share the returned `Arc` across all
/// connection handlers and bulk transfer sessions.
///
/// ```ignore
/// let encrypt_pool = build_encrypt_pool();
/// // pass encrypt_pool to connection handlers via DaemonContext
/// ```
pub fn build_encrypt_pool() -> Arc<ThreadPool> {
    let workers = encrypt_workers();
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .thread_name(|i| format!("rekindle-encrypt-{i}"))
            .start_handler(|idx| {
                // Pin encrypt workers to cores starting at offset 2,
                // leaving cores 0-1 for the Tokio I/O runtime.
                // On machines with fewer than workers+2 cores, pinning
                // is skipped (the OS scheduler handles placement).
                #[cfg(target_os = "linux")]
                {
                    let target_cpu = idx + 2;
                    // SAFETY: zeroed cpu_set_t is valid (all-zeros = no CPUs selected).
                    // CPU_SET sets one bit. sched_setaffinity with a valid set and
                    // pid=0 (current thread) cannot cause UB — returns -1 on failure.
                    unsafe {
                        let mut set = std::mem::zeroed::<libc::cpu_set_t>();
                        libc::CPU_SET(target_cpu, &mut set);
                        let result = libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &raw const set);
                        if result != 0 {
                            // Pinning failed (core doesn't exist or no permission).
                            // Non-fatal — the OS scheduler will place the thread.
                        }
                    }
                }
                let _ = idx;
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
}
