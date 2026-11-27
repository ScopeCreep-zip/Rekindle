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
#[path = "retry_tests.rs"]
mod tests;
