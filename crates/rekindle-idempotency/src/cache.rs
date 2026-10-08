//! [`IdempotencyCache`] — LRU + TTL cache keyed on UUID v7 per gesture.
//!
//! Wraps any `async FnOnce() -> T` so duplicate calls with the same
//! key share ONE execution: the first call runs the closure, every
//! subsequent caller awaits the cached result. The cache itself uses
//! `moka::future::Cache` for thread-safe async-aware get/insert.
//!
//! ## Concurrency model
//!
//! The plan's stress scenario is "click Send 10× as fast as possible".
//! That dispatches 10 simultaneous Tauri commands, each holding the
//! same idempotency key. `moka::Cache::get_with` ensures exactly ONE
//! task executes the closure; the other 9 await the in-flight future
//! and receive the same cloned result.
//!
//! ## Lifetimes
//!
//! Entries expire from the cache via:
//! - **LRU eviction** when `max_entries` is exceeded.
//! - **TTL expiry** after `ttl` since insert.
//!
//! A reasonable default is 4096 entries × 60 s TTL — long enough to
//! cover transient retries (network blip, Tauri renderer reload),
//! short enough to not hold large response payloads forever.

#![forbid(unsafe_code)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use uuid::Uuid;

/// LRU + TTL cache keyed on UUID v7. `T` is the cached response type
/// (e.g. `Result<DmAck, String>`). The clone bound is from `moka`'s
/// `get_with` contract — the cached value is cloned to every waiting
/// caller.
///
/// Construct once per application lifetime; share via `Arc`. The cache
/// is internally `Send + Sync`.
pub struct IdempotencyCache<T: Clone + Send + Sync + 'static> {
    inner: Cache<Uuid, T>,
}

impl<T: Clone + Send + Sync + 'static> IdempotencyCache<T> {
    /// Construct with explicit capacity + TTL.
    ///
    /// `max_entries`: how many distinct idempotency keys to track
    /// before LRU eviction kicks in.
    /// `ttl`: how long after insert an entry stays cacheable.
    #[must_use]
    pub fn new(max_entries: u64, ttl: Duration) -> Self {
        Self {
            inner: Cache::builder()
                .max_capacity(max_entries)
                .time_to_live(ttl)
                .build(),
        }
    }

    /// Reasonable defaults per the plan: 4096 entries × 60 s TTL.
    /// Use this in production wiring.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(4096, Duration::from_secs(60))
    }

    /// Run `f()` if no value is cached under `key`, otherwise return the
    /// cached value. Concurrent calls with the same key all await the
    /// SAME in-flight future; the closure runs at most once per key.
    ///
    /// The closure is `FnOnce` returning a `Future` — perfect for
    /// wrapping a Tauri command body that performs side effects (DM
    /// send, channel create, etc.). The cached value (typically a
    /// `Result<Ack, String>`) is cloned to every concurrent caller.
    pub async fn wrap<F, Fut>(&self, key: Uuid, f: F) -> T
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = T> + Send,
    {
        // moka's get_with handles the "race winner runs closure, others
        // await" coordination natively.
        self.inner
            .get_with(key, async {
                let result = f().await;
                tracing::trace!(?key, "idempotency cache miss; closure executed");
                result
            })
            .await
    }

    /// Diagnostic: number of entries currently cached.
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Diagnostic: drop a specific key from the cache (e.g., for tests).
    pub async fn invalidate(&self, key: Uuid) {
        self.inner.invalidate(&key).await;
    }
}

impl<T: Clone + Send + Sync + 'static> Default for IdempotencyCache<T> {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Type-erased Arc wrapper used by callers that want to share one
/// cache across many tasks without re-templating on `T`.
pub type SharedIdempotencyCache<T> = Arc<IdempotencyCache<T>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Closure runs at most once for a given key, even under
    /// concurrent calls — the plan's "click Send 10× → 1 invocation"
    /// guarantee.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_same_key_runs_closure_once() {
        let cache: IdempotencyCache<u32> = IdempotencyCache::new(64, Duration::from_secs(60));
        let cache = Arc::new(cache);
        let invocations = Arc::new(AtomicU32::new(0));
        let key = Uuid::now_v7();

        let mut tasks = Vec::with_capacity(10);
        for _ in 0..10 {
            let c = Arc::clone(&cache);
            let inv = Arc::clone(&invocations);
            tasks.push(tokio::spawn(async move {
                c.wrap(key, || async move {
                    inv.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    42
                })
                .await
            }));
        }
        let mut results = Vec::new();
        for t in tasks {
            results.push(t.await.expect("task panicked"));
        }
        assert_eq!(results.len(), 10);
        assert!(
            results.iter().all(|&r| r == 42),
            "all callers see the same value"
        );
        assert_eq!(
            invocations.load(Ordering::Relaxed),
            1,
            "closure must execute exactly once across 10 concurrent calls",
        );
    }

    /// Different keys run the closure independently.
    #[tokio::test]
    async fn different_keys_run_independent_closures() {
        let cache: IdempotencyCache<u32> = IdempotencyCache::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let c1 = Arc::clone(&counter);
        let v1 = cache
            .wrap(Uuid::now_v7(), || async move {
                c1.fetch_add(1, Ordering::Relaxed);
                1
            })
            .await;
        let c2 = Arc::clone(&counter);
        let v2 = cache
            .wrap(Uuid::now_v7(), || async move {
                c2.fetch_add(1, Ordering::Relaxed);
                2
            })
            .await;
        assert_eq!(v1, 1);
        assert_eq!(v2, 2);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    /// Sequential calls with the same key return the SAME cached
    /// value — the second call doesn't re-execute.
    #[tokio::test]
    async fn sequential_same_key_returns_cached() {
        let cache: IdempotencyCache<u32> = IdempotencyCache::with_defaults();
        let key = Uuid::now_v7();
        let counter = Arc::new(AtomicU32::new(0));

        let c1 = Arc::clone(&counter);
        let v1 = cache
            .wrap(key, || async move {
                c1.fetch_add(1, Ordering::Relaxed);
                100
            })
            .await;
        let c2 = Arc::clone(&counter);
        let v2 = cache
            .wrap(key, || async move {
                c2.fetch_add(1, Ordering::Relaxed);
                200 // would-be different value if it ran
            })
            .await;
        assert_eq!(v1, 100);
        assert_eq!(v2, 100, "second call must return cached value, not 200");
        assert_eq!(counter.load(Ordering::Relaxed), 1, "closure must run once");
    }

    /// After TTL expiry the closure runs again on next call with the
    /// same key.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ttl_expiry_allows_re_execution() {
        let cache: IdempotencyCache<u32> = IdempotencyCache::new(64, Duration::from_millis(100));
        let key = Uuid::now_v7();
        let counter = Arc::new(AtomicU32::new(0));

        let c1 = Arc::clone(&counter);
        let _ = cache
            .wrap(key, || async move {
                c1.fetch_add(1, Ordering::Relaxed);
                1
            })
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let c2 = Arc::clone(&counter);
        let _ = cache
            .wrap(key, || async move {
                c2.fetch_add(1, Ordering::Relaxed);
                2
            })
            .await;
        assert_eq!(
            counter.load(Ordering::Relaxed),
            2,
            "closure must re-run after TTL expiry",
        );
    }

    /// Errors are cached too — a failed send won't silently re-execute
    /// on retry (which is the intended idempotency semantic).
    #[tokio::test]
    async fn errors_are_cached() {
        let cache: IdempotencyCache<Result<u32, String>> = IdempotencyCache::with_defaults();
        let key = Uuid::now_v7();
        let counter = Arc::new(AtomicU32::new(0));

        let c1 = Arc::clone(&counter);
        let r1 = cache
            .wrap(key, || async move {
                c1.fetch_add(1, Ordering::Relaxed);
                Err::<u32, _>("oops".to_string())
            })
            .await;
        let c2 = Arc::clone(&counter);
        let r2 = cache
            .wrap(key, || async move {
                c2.fetch_add(1, Ordering::Relaxed);
                Ok::<u32, String>(42)
            })
            .await;
        assert_eq!(r1.as_ref().err(), Some(&"oops".to_string()));
        assert_eq!(
            r2.as_ref().err(),
            Some(&"oops".to_string()),
            "error must be cached and returned on retry",
        );
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    /// Invalidate drops a key so the next call re-executes.
    #[tokio::test]
    async fn invalidate_allows_re_execution() {
        let cache: IdempotencyCache<u32> = IdempotencyCache::with_defaults();
        let key = Uuid::now_v7();
        let counter = Arc::new(AtomicU32::new(0));

        let c1 = Arc::clone(&counter);
        let _ = cache
            .wrap(key, || async move {
                c1.fetch_add(1, Ordering::Relaxed);
                1
            })
            .await;
        cache.invalidate(key).await;
        let c2 = Arc::clone(&counter);
        let _ = cache
            .wrap(key, || async move {
                c2.fetch_add(1, Ordering::Relaxed);
                2
            })
            .await;
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    /// `with_defaults` constructs a usable cache.
    #[test]
    fn defaults_construct_usable_cache() {
        let _: IdempotencyCache<u32> = IdempotencyCache::with_defaults();
    }
}
