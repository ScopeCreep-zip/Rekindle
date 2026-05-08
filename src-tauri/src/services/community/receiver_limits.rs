//! M10.4 — receiver-side enforcement of per-sender gossip rate floor
//! (architecture §20.2 line 2585) and per-channel slowmode (§28.7 line
//! 3187).
//!
//! The send-side enforces these limits for honest clients. The receive-
//! side enforces them for everyone — a modified client that bypasses
//! its own send-side limiter still has its messages dropped by every
//! honest peer. Reader-validates symmetry per chiral §16.3.
//!
//! Both maps are pruned in-line when they exceed soft caps. Pruning
//! drops entries that have been idle longer than `IDLE_TTL_SECS` so
//! short-lived bursts (a sender talks for 30s then stops) don't pin the
//! map indefinitely, and so a single noisy community can't push other
//! communities' entries out via LRU pressure.

use std::collections::HashMap;
use std::time::Instant;

use rekindle_gossip::rate_limit::TokenBucket;
use rekindle_governance::permissions::compute_permissions;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::permissions::BYPASS_SLOWMODE;

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
fn default_bucket() -> TokenBucket {
    TokenBucket::ten_per_second()
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
    let bucket = limits.entry(key).or_insert_with(default_bucket);
    let accepted = bucket.try_consume(1);
    if limits.len() > SOFT_CAP {
        prune_idle_buckets(&mut limits);
    }
    accepted
}

/// Receiver-side slowmode check (architecture §28.7). Looks up the
/// channel's `slowmode_seconds` from `CommunityState.channels`,
/// consults the sender's effective permissions for `BYPASS_SLOWMODE`,
/// and rejects sub-window sends.
///
/// Returns `true` to accept, `false` to silently drop.
///
/// On accept, updates the per-(community, channel, sender) last-seen
/// timestamp so the next message in the window is rejected.
pub fn check_slowmode(
    state: &AppState,
    community_id: &str,
    channel_id_hex: &str,
    sender: &str,
    now_secs: u64,
) -> bool {
    let (slowmode_seconds, sender_bypasses) = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            // Unknown community — let the downstream handler decide.
            return true;
        };
        let slowmode = community
            .channels
            .iter()
            .find(|ch| ch.id == channel_id_hex)
            .and_then(|ch| ch.slowmode_seconds)
            .unwrap_or(0);
        let bypass = sender_has_bypass(community, channel_id_hex, sender, now_secs);
        (slowmode, bypass)
    };

    if slowmode_seconds == 0 || sender_bypasses {
        return true;
    }

    let mut last = state.channel_last_received.lock();
    let key = (
        community_id.to_string(),
        channel_id_hex.to_string(),
        sender.to_string(),
    );
    if let Some(prev) = last.get(&key) {
        if now_secs.saturating_sub(*prev) < u64::from(slowmode_seconds) {
            return false;
        }
    }
    last.insert(key, now_secs);
    if last.len() > SOFT_CAP {
        prune_idle_timestamps(&mut last, now_secs);
    }
    true
}

fn sender_has_bypass(
    community: &crate::state::CommunityState,
    channel_id_hex: &str,
    sender: &str,
    now_secs: u64,
) -> bool {
    let Some(governance_state) = community.governance_state.as_ref() else {
        return false;
    };
    let Some(sender_pseudo) = decode_pseudonym(sender) else {
        return false;
    };
    let channel_id_opt = decode_channel_id(channel_id_hex);
    let perms = compute_permissions(
        &sender_pseudo,
        channel_id_opt.as_ref(),
        governance_state,
        now_secs,
    );
    (perms & BYPASS_SLOWMODE) != 0
}

fn decode_pseudonym(hex_str: &str) -> Option<PseudonymKey> {
    let bytes = hex::decode(hex_str).ok()?;
    let array: [u8; 32] = bytes.try_into().ok()?;
    Some(PseudonymKey(array))
}

fn decode_channel_id(hex_str: &str) -> Option<ChannelId> {
    let bytes = hex::decode(hex_str).ok()?;
    let array: [u8; 16] = bytes.try_into().ok()?;
    Some(ChannelId(array))
}

fn prune_idle_buckets(buckets: &mut HashMap<(String, String), TokenBucket>) {
    let now = Instant::now();
    buckets.retain(|_, bucket| {
        let idle = now.saturating_duration_since(bucket.last_refill());
        idle.as_secs() < IDLE_TTL_SECS
    });
}

fn prune_idle_timestamps(map: &mut HashMap<(String, String, String), u64>, now_secs: u64) {
    map.retain(|_, last| now_secs.saturating_sub(*last) < IDLE_TTL_SECS);
}
