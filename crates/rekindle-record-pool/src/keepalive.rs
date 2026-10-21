//! Record keepalive: rehydrate a community's records on a jittered cadence
//! (architecture §14.1, mutual aid: usage is maintenance; plan C7, 12.K and
//! C7.8b). One loop for every host; the host supplies which records.
//!
//! Wave 8 P8.1: a fixed cadence is itself a fingerprint, since an observer
//! correlating Veilid DHT reads can attribute warming events to one peer.
//! So the interval carries uniform jitter (±20 % of the base) and each
//! community starts at a random offset within the first window, and two
//! clients warming the same record never burst in lockstep.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::RecordPort;
use rekindle_utils::RandomSource;
use tokio_util::sync::CancellationToken;

/// The base interval between warmings.
pub const BASE_INTERVAL: Duration = Duration::from_secs(300);
/// Uniform jitter on top of [`BASE_INTERVAL`], in seconds either way.
const INTERVAL_JITTER_SECS: i64 = 60;

/// The next warming delay: [`BASE_INTERVAL`] ± up to 60 s, from the host's
/// generator (plan C9.2).
#[must_use]
pub fn next_warming_delay(rng: &dyn RandomSource) -> Duration {
    let span = INTERVAL_JITTER_SECS.unsigned_abs() * 2 + 1;
    let offset = i64::try_from(rng.below(span)).unwrap_or(0) - INTERVAL_JITTER_SECS;
    Duration::from_secs(BASE_INTERVAL.as_secs().saturating_add_signed(offset))
}

/// The per-community start offset, in `[0, BASE_INTERVAL)`, from the host's
/// generator.
#[must_use]
pub fn initial_offset(rng: &dyn RandomSource) -> Duration {
    Duration::from_secs(rng.below(BASE_INTERVAL.as_secs()))
}

/// Rehydrate one record: a re-open of a local record is Veilid's only
/// trigger for rehydration (`open_record.rs:27-44`), and the pool re-opens
/// with the record's sticky writer, so nothing is downgraded; then it pulls
/// any subkey the network holds newer. A table hit while the record is held.
async fn rehydrate(pool: &dyn RecordPort, key: &str) {
    let lease = match pool.acquire(key, None).await {
        Ok(lease) => lease,
        Err(e) => {
            tracing::debug!(key, error = %e, "keepalive: record not open");
            return;
        }
    };
    if let Err(e) = pool.rehydrate(lease).await {
        tracing::debug!(key, error = %e, "keepalive: rehydrate failed");
    }
    pool.release(lease).await;
}

/// Run the keepalive until `stop`: after a random start offset (from `rng`,
/// the host's generator), rehydrate
/// every record `keys` names, then wait a jittered interval, and again.
/// `pool` is the session's pool while there is one; `keys` is `None` once
/// the community is gone. Leaving the community (or the session) cancels
/// `stop`; the pool runs each Veilid call on its own scope, so leaving
/// mid-cycle drops none (plan C4.L1).
pub async fn run(
    pool: impl Fn() -> Option<Arc<dyn RecordPort>>,
    keys: impl Fn() -> Option<Vec<String>>,
    rng: Arc<dyn RandomSource>,
    stop: CancellationToken,
) {
    if stop
        .run_until_cancelled(tokio::time::sleep(initial_offset(rng.as_ref())))
        .await
        .is_none()
    {
        return;
    }
    loop {
        let Some(keys) = keys() else {
            return;
        };
        if let Some(pool) = pool() {
            let count = keys.len();
            for key in keys {
                if stop
                    .run_until_cancelled(rehydrate(&*pool, &key))
                    .await
                    .is_none()
                {
                    return;
                }
            }
            tracing::info!(records = count, "keepalive: community records rehydrated");
        }
        if stop
            .run_until_cancelled(tokio::time::sleep(next_warming_delay(rng.as_ref())))
            .await
            .is_none()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cadence_stays_within_its_jitter_and_the_start_within_one_window() {
        for _ in 0..200 {
            let delay = next_warming_delay(&rekindle_utils::OsRandom).as_secs();
            assert!((240..=360).contains(&delay), "delay {delay}");
            assert!(initial_offset(&rekindle_utils::OsRandom) < BASE_INTERVAL);
        }
    }
}
