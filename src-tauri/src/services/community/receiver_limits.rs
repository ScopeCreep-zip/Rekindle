//! M10.4 — receiver-side enforcement of per-sender gossip rate floor
//! (architecture §20.2 line 2585). Per-channel slowmode (§28.7 line 3187)
//! is the Host ingest gate's (`rekindle_channel::ingest`, plan E3.1 F3).
//!
//! The send-side enforces these limits for honest clients. The receive-
//! side enforces them for everyone — a modified client that bypasses
//! its own send-side limiter still has its messages dropped by every
//! honest peer. Reader-validates symmetry per chiral §16.3.
//!
//! The map is pruned in-line when they exceed soft caps. Pruning
//! drops entries that have been idle longer than `IDLE_TTL_SECS` so
//! short-lived bursts (a sender talks for 30s then stops) don't pin the
//! map indefinitely, and so a single noisy community can't push other
//! communities' entries out via LRU pressure.

use std::collections::HashMap;
use std::time::Instant;

use rekindle_gossip::rate_limit::TokenBucket;

use crate::state::AppState;

/// Soft cap. When either map exceeds this, prune entries idle > TTL.
/// 10_000 chosen to stay well under the spawn-tasks cap and to keep the
/// HashMap's footprint under ~1 MiB at typical entry sizes.
const SOFT_CAP: usize = 10_000;

/// Buckets / timestamps idle longer than this are pruned.
const IDLE_TTL_SECS: u64 = 60;

/// Per-sender gossip rate floor (architecture §20.2). 10 messages per
/// second sustained, 10 burst. The chiral spec calls this the "hard
/// floor" — even a malicious client cannot exceed it because honest
/// receivers drop the excess.
fn default_bucket(now: Instant) -> TokenBucket {
    TokenBucket::ten_per_second(now)
}

/// Receiver-side check before any gossip dispatch. Returns `true` if the
/// envelope should be processed; `false` if the sender has exceeded the
/// per-(community, sender) floor and the envelope should be silently
/// dropped per §20.2.
///
/// Side effect: consumes one token from the sender's bucket on accept.
pub fn check_gossip_rate(state: &AppState, community_id: &str, sender: &str) -> bool {
    let mut limits = state.gossip_rate_limits.lock();
    let key = (community_id.to_string(), sender.to_string());
    let now = Instant::now();
    let bucket = limits.entry(key).or_insert_with(|| default_bucket(now));
    let accepted = bucket.try_consume_at(1, now);
    if limits.len() > SOFT_CAP {
        prune_idle_buckets(&mut limits);
    }
    accepted
}

fn prune_idle_buckets(buckets: &mut HashMap<(String, String), TokenBucket>) {
    let now = Instant::now();
    buckets.retain(|_, bucket| {
        let idle = now.saturating_duration_since(bucket.last_refill());
        idle.as_secs() < IDLE_TTL_SECS
    });
}
