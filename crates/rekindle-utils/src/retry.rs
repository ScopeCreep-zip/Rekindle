//! Bounded retry with backoff — the one retry loop for the workspace.
//!
//! Before this module existed the same loop was hand-rolled four times
//! (two DHT open retries, since replaced by the record pool's one retry
//! layer, and two private-route allocators in the Tauri host), each with
//! its own attempt counting, sleeping, and logging.
//! Only two things ever differed per site: the backoff shape and the
//! "is this error worth retrying" predicate — so those are the
//! parameters, and everything else lives here once.
//!
//! Feature-gated behind `retry` (needs `tokio` + `tracing`); the rest of
//! `rekindle-utils` stays std-only.

use std::future::Future;
use std::time::Duration;

/// Backoff shape for [`retry_with_backoff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts (clamped to at least 1).
    pub attempts: u32,
    /// Delay before the first retry. `Duration::ZERO` disables sleeping
    /// entirely (tests).
    pub initial_delay: Duration,
    /// Ceiling for the doubling backoff. Equal to `initial_delay` for a
    /// fixed-interval retry.
    pub max_delay: Duration,
}

impl RetryPolicy {
    /// Fixed-interval policy: `attempts` tries, `delay` between each.
    pub const fn fixed(attempts: u32, delay: Duration) -> Self {
        Self {
            attempts,
            initial_delay: delay,
            max_delay: delay,
        }
    }

    /// Exponential policy: delay doubles from `initial` per retry,
    /// clamped to `max`.
    pub const fn exponential(attempts: u32, initial: Duration, max: Duration) -> Self {
        Self {
            attempts,
            initial_delay: initial,
            max_delay: max,
        }
    }
}

/// Retry `op` while it fails with an error `is_transient` accepts, up to
/// `policy.attempts` tries with the policy's backoff between them.
///
/// - Success returns immediately.
/// - A non-transient (hard) error returns immediately — retrying is
///   pointless or wrong for it.
/// - When the budget is exhausted the LAST transient error is returned,
///   so the caller can decide what exhaustion means (e.g. "record is
///   genuinely gone — recreate").
///
/// `label` names the operation in the per-retry debug log.
pub async fn retry_with_backoff<T, E, F, Fut>(
    policy: RetryPolicy,
    label: &str,
    is_transient: impl FnMut(&E) -> bool,
    op: F,
) -> Result<T, E>
where
    E: std::fmt::Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let never = tokio_util::sync::CancellationToken::new();
    retry_with_backoff_until(policy, label, is_transient, &never, op)
        .await
        .expect("a token nobody cancels never stops the retry")
}

/// [`retry_with_backoff`] for session work: stops when `stop` is
/// cancelled, before the next attempt or during a backoff sleep, and
/// returns `None`. An attempt already running finishes: the operations
/// retried here are Veilid calls, which have no cancellation (plan C4.L1).
pub async fn retry_with_backoff_until<T, E, F, Fut>(
    policy: RetryPolicy,
    label: &str,
    mut is_transient: impl FnMut(&E) -> bool,
    stop: &tokio_util::sync::CancellationToken,
    mut op: F,
) -> Option<Result<T, E>>
where
    E: std::fmt::Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let attempts = policy.attempts.max(1);
    let mut delay = policy.initial_delay;

    for attempt in 1..=attempts {
        if stop.is_cancelled() {
            return None;
        }
        match op().await {
            Ok(v) => return Some(Ok(v)),
            Err(e) if is_transient(&e) => {
                if attempt == attempts {
                    return Some(Err(e));
                }
                tracing::debug!(
                    label,
                    attempt,
                    attempts,
                    delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                    error = %e,
                    "transient failure; retrying"
                );
                if !delay.is_zero()
                    && stop
                        .run_until_cancelled(tokio::time::sleep(delay))
                        .await
                        .is_none()
                {
                    return None;
                }
                delay = delay.saturating_mul(2).min(policy.max_delay);
            }
            Err(e) => return Some(Err(e)),
        }
    }
    unreachable!("loop returns on success, hard error, or final attempt")
}

#[cfg(test)]
mod tests {
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
}
