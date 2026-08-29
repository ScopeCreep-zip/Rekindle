//! Bounded retry with backoff — the one retry loop for the workspace.
//!
//! Before this module existed the same loop was hand-rolled four times
//! (`rekindle-protocol::dht::retry_on_unreachable`, the transport DHT
//! `open_with_retry`, and two private-route allocators in the Tauri
//! host), each with its own attempt counting, sleeping, and logging.
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
    mut is_transient: impl FnMut(&E) -> bool,
    mut op: F,
) -> Result<T, E>
where
    E: std::fmt::Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let attempts = policy.attempts.max(1);
    let mut delay = policy.initial_delay;

    for attempt in 1..=attempts {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if is_transient(&e) => {
                if attempt == attempts {
                    return Err(e);
                }
                tracing::debug!(
                    label,
                    attempt,
                    attempts,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "transient failure; retrying"
                );
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                delay = delay.saturating_mul(2).min(policy.max_delay);
            }
            Err(e) => return Err(e),
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
        let res: Result<&str, String> = retry_with_backoff(ZERO, "t", |_| true, || {
            let n = calls.get();
            calls.set(n + 1);
            async move {
                if n < 2 {
                    Err(format!("transient {n}"))
                } else {
                    Ok("done")
                }
            }
        })
        .await;
        assert_eq!(res.unwrap(), "done");
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn hard_error_short_circuits() {
        let calls = Cell::new(0u32);
        let res: Result<(), String> =
            retry_with_backoff(ZERO, "t", |e: &String| e.starts_with("soft"), || {
                calls.set(calls.get() + 1);
                async { Err("hard failure".to_string()) }
            })
            .await;
        assert_eq!(res.unwrap_err(), "hard failure");
        assert_eq!(calls.get(), 1, "hard error must not be retried");
    }

    #[tokio::test]
    async fn exhaustion_returns_last_transient_error() {
        let calls = Cell::new(0u32);
        let res: Result<(), String> = retry_with_backoff(ZERO, "t", |_| true, || {
            let n = calls.get();
            calls.set(n + 1);
            async move { Err(format!("transient {n}")) }
        })
        .await;
        assert_eq!(res.unwrap_err(), "transient 4");
        assert_eq!(calls.get(), 5, "budget of 5 attempts fully spent");
    }

    #[tokio::test]
    async fn zero_attempts_clamps_to_one() {
        let calls = Cell::new(0u32);
        let res: Result<(), String> =
            retry_with_backoff(RetryPolicy::fixed(0, Duration::ZERO), "t", |_| true, || {
                calls.set(calls.get() + 1);
                async { Err("nope".to_string()) }
            })
            .await;
        assert!(res.is_err());
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn exponential_backoff_doubles_and_clamps() {
        // 4 attempts: sleeps of 100ms, 200ms, 300ms(clamped) = 600ms total.
        let policy = RetryPolicy::exponential(
            4,
            Duration::from_millis(100),
            Duration::from_millis(300),
        );
        let start = tokio::time::Instant::now();
        let res: Result<(), String> =
            retry_with_backoff(policy, "t", |_| true, || async { Err("t".to_string()) }).await;
        assert!(res.is_err());
        assert_eq!(start.elapsed(), Duration::from_millis(600));
    }
}
