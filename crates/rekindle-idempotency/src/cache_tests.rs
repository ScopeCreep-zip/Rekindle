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
