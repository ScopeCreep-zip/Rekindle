//! Record keepalive: rehydrate a community's records on a jittered cadence
//! (architecture §14.1, mutual aid: usage is maintenance; plan C7, 12.K,
//! C7.8b and VU decision 7). One loop for every host; the host supplies
//! which records.
//!
//! Veilid 0.5.8 queues a rehydration on a re-open only when the open asks
//! for it ([`OpenIntent::Hold`]), the record was not created by this node
//! this session (`locally_created_records`), and the last rehydration of it
//! finished at least 60 RPC timeouts ago: 300 s at the 5 s default
//! (`storage_manager/open_record.rs:62-81`, `mod.rs:178-198`,
//! `tasks/rehydrate_records.rs:50-52`). So the interval is never shorter
//! than that window ([`BASE_INTERVAL`], 300 s) and no cycle is a no-op.
//! Recorded limit: a record this node created this session is never
//! rehydrated by a re-open in that session; its owner's own writes carry
//! it, and the pool's inspect-and-fetch pass still runs on its unwatched
//! subkeys.
//!
//! Plan KA1 (amends VU decision 7, D4): the inspect-and-fetch covers only
//! the subkeys no armed watch of this node covers; a watched subkey's
//! changes arrive through its watch, so the keepalive never inspects a
//! watched record on a timer. The `Hold` re-open runs for every record.
//!
//! Wave 8 P8.1: a fixed cadence is itself a fingerprint, since an observer
//! correlating Veilid DHT reads can attribute warming events to one peer.
//! So the interval carries uniform jitter (up to 20 % of the base, from the
//! host's generator) and each community starts at a random offset within
//! the first window, and two clients warming the same record never burst
//! in lockstep.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{OpenIntent, RecordPort};
use rekindle_utils::RandomSource;
use tokio_util::sync::CancellationToken;

/// The shortest interval between warmings: Veilid's rehydration recheck
/// window, 60 × the 5 s RPC timeout (0.5.8 `storage_manager/mod.rs:198`), which
/// Rekindle keeps (`veilid_config.rs` `the_call_timeout_is_veilids_rpc_timeout`).
pub const BASE_INTERVAL: Duration = Duration::from_secs(300);
/// Uniform jitter added to [`BASE_INTERVAL`], in seconds, never subtracted.
const INTERVAL_JITTER_SECS: u64 = 60;

/// The next warming delay: [`BASE_INTERVAL`] plus up to 60 s, from the
/// host's generator (plan C9.2). The recheck window counts from when the
/// last rehydration finished, so the jitter only ever lengthens it.
#[must_use]
pub fn next_warming_delay(rng: &dyn RandomSource) -> Duration {
    BASE_INTERVAL + Duration::from_secs(rng.below(INTERVAL_JITTER_SECS + 1))
}

/// The per-community start offset, in `[0, BASE_INTERVAL)`, from the host's
/// generator.
#[must_use]
pub fn initial_offset(rng: &dyn RandomSource) -> Duration {
    Duration::from_secs(rng.below(BASE_INTERVAL.as_secs()))
}

/// Rehydrate one record: the borrow is [`OpenIntent::Hold`] (warming is
/// custody), and [`RecordPort::rehydrate`] re-opens with the record's
/// sticky writer as `Hold`, the only app-side trigger of Veilid's
/// rehydration (queued when due, see the module notes), so nothing is
/// downgraded; then it pulls any subkey the network holds newer. A table
/// hit while the record is held.
async fn rehydrate(pool: &dyn RecordPort, key: &str) {
    let lease = match pool.acquire(key, None, OpenIntent::Hold).await {
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

    /// The cadence never undercuts Veilid's 300 s rehydration recheck
    /// window (0.5.8 `storage_manager/mod.rs:198`), so no warming is a no-op.
    #[test]
    fn the_cadence_stays_within_its_jitter_and_the_start_within_one_window() {
        for _ in 0..200 {
            let delay = next_warming_delay(&rekindle_utils::OsRandom).as_secs();
            assert!((300..=360).contains(&delay), "delay {delay}");
            assert!(initial_offset(&rekindle_utils::OsRandom) < BASE_INTERVAL);
        }
    }
}
