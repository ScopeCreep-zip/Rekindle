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

    /// Like [`Self::wrap`], but only an `Ok` value is cached. Concurrent
    /// callers with the same key share one run of `f` and all see its
    /// outcome; an `Err` is handed to them and then forgotten, so a later
    /// call runs `f` again.
    ///
    /// # Errors
    /// `f`'s error, shared among the concurrent callers.
    pub async fn try_wrap<F, Fut, E>(&self, key: Uuid, f: F) -> Result<T, Arc<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        E: Send + Sync + 'static,
    {
        self.inner.try_get_with(key, async { f().await }).await
    }

    /// The value cached under `key`, if any; runs nothing.
    pub async fn get(&self, key: Uuid) -> Option<T> {
        self.inner.get(&key).await
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
#[path = "cache_tests.rs"]
mod tests;
