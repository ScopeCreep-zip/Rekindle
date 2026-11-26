use super::*;
use std::cell::Cell;

const ZERO: RetryPolicy = RetryPolicy::fixed(5, Duration::ZERO);

#[tokio::test]
async fn succeeds_first_try() {
    let res: Result<u32, String> =
        retry_with_backoff(ZERO, "t", |_| true, || async { Ok(7) }).await;
    assert_eq!(res.unwrap(), 7);
}

#[tokio::test]
async fn recovers_after_transient_failures() {
    let calls = Cell::new(0u32);
    let res: Result<&str, String> = retry_with_backoff(
        ZERO,
        "t",
        |_| true,
        || {
            let n = calls.get();
            calls.set(n + 1);
            async move {
                if n < 2 {
                    Err(format!("transient {n}"))
                } else {
                    Ok("done")
                }
            }
        },
    )
    .await;
    assert_eq!(res.unwrap(), "done");
    assert_eq!(calls.get(), 3);
}

#[tokio::test]
async fn hard_error_short_circuits() {
    let calls = Cell::new(0u32);
    let res: Result<(), String> = retry_with_backoff(
        ZERO,
        "t",
        |e: &String| e.starts_with("soft"),
        || {
            calls.set(calls.get() + 1);
            async { Err("hard failure".to_string()) }
        },
    )
    .await;
    assert_eq!(res.unwrap_err(), "hard failure");
    assert_eq!(calls.get(), 1, "hard error must not be retried");
}

#[tokio::test]
async fn exhaustion_returns_last_transient_error() {
    let calls = Cell::new(0u32);
    let res: Result<(), String> = retry_with_backoff(
        ZERO,
        "t",
        |_| true,
        || {
            let n = calls.get();
            calls.set(n + 1);
            async move { Err(format!("transient {n}")) }
        },
    )
    .await;
    assert_eq!(res.unwrap_err(), "transient 4");
    assert_eq!(calls.get(), 5, "budget of 5 attempts fully spent");
}

#[tokio::test]
async fn zero_attempts_clamps_to_one() {
    let calls = Cell::new(0u32);
    let res: Result<(), String> = retry_with_backoff(
        RetryPolicy::fixed(0, Duration::ZERO),
        "t",
        |_| true,
        || {
            calls.set(calls.get() + 1);
            async { Err("nope".to_string()) }
        },
    )
    .await;
    assert!(res.is_err());
    assert_eq!(calls.get(), 1);
}

#[tokio::test(start_paused = true)]
async fn exponential_backoff_doubles_and_clamps() {
    // 4 attempts: sleeps of 100ms, 200ms, 300ms(clamped) = 600ms total.
    let policy =
        RetryPolicy::exponential(4, Duration::from_millis(100), Duration::from_millis(300));
    let start = tokio::time::Instant::now();
    let res: Result<(), String> =
        retry_with_backoff(policy, "t", |_| true, || async { Err("t".to_string()) }).await;
    assert!(res.is_err());
    assert_eq!(start.elapsed(), Duration::from_millis(600));
}

#[tokio::test]
async fn a_cancelled_token_stops_before_the_first_attempt() {
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    let res: Option<Result<u32, String>> =
        retry_with_backoff_until(ZERO, "t", |_| true, &stop, || async { Ok(1) }).await;
    assert!(res.is_none());
}

#[tokio::test(start_paused = true)]
async fn cancelling_during_backoff_stops_the_retry() {
    let stop = tokio_util::sync::CancellationToken::new();
    let calls = Cell::new(0u32);
    let canceller = stop.clone();
    let res: Option<Result<u32, String>> = retry_with_backoff_until(
        RetryPolicy::fixed(5, Duration::from_secs(3)),
        "t",
        |_| true,
        &stop,
        || {
            calls.set(calls.get() + 1);
            canceller.cancel();
            async { Err("transient".to_string()) }
        },
    )
    .await;
    assert!(res.is_none());
    assert_eq!(calls.get(), 1, "no attempt after the stop");
}
