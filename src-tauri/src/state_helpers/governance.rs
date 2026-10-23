//! v2.0 CRDT governance state read/write helpers.

use rekindle_types::lamport::{LamportClock, LamportError};
use std::sync::Arc;

use crate::state::AppState;

/// Get a clone of the cached CRDT governance state for a community.
///
/// Returns `None` if the community doesn't exist or governance state isn't loaded yet.
pub fn governance_state(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<rekindle_governance::state::GovernanceState> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|cs| cs.governance_state.clone())
}

/// Get the governance record DHT key for a community.
pub fn governance_key(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|cs| cs.governance_key.clone())
}

/// Tick the community's message clock and return the new value.
pub fn next_message_lamport(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<u64, LamportError> {
    tick(state, community_id, |cs| &mut cs.message_clock)
}

/// Observe a received message's Lamport timestamp: the message clock
/// advances past it, by at most `MAX_LAMPORT_DRIFT + 1` (clamped, never
/// rejected — the message orders by its own timestamp).
pub fn merge_message_lamport(state: &Arc<AppState>, community_id: &str, received: u64) {
    if let Some(cs) = state.communities.write().get_mut(community_id) {
        let mut clock = LamportClock::new(cs.message_clock);
        cs.message_clock = clock.merge(received);
    }
}

fn tick(
    state: &Arc<AppState>,
    community_id: &str,
    clock_of: impl FnOnce(&mut crate::state::CommunityState) -> &mut u64,
) -> Result<u64, LamportError> {
    let mut communities = state.communities.write();
    let cs = communities
        .get_mut(community_id)
        .ok_or(LamportError::UnknownCommunity)?;
    let value = clock_of(cs);
    let next = LamportClock::new(*value).increment()?;
    *value = next;
    Ok(next)
}

/// Compute effective permissions for an arbitrary pseudonym in a
/// community channel, given the community's state directly (for callers
/// that already hold a `&CommunityState`, e.g. from inside a
/// `state.communities.read()` guard, and shouldn't re-lock to get one).
///
/// Returns `None` when governance state isn't loaded yet or
/// `pseudonym_hex` doesn't decode to a 32-byte key. Centralizes what was
/// previously reimplemented, by hand, at five separate call sites —
/// `community/message_notifications.rs`, `community/link_previews.rs`,
/// `community/receiver_limits.rs`, `veilid/control.rs`, and
/// `veilid/control_sync.rs` — each independently doing its own
/// hex-decode → `PseudonymKey` → `compute_permissions`. Callers that want
/// a zero-permission default on failure (the common case, matching the
/// old reimplementations) call `.unwrap_or(0)`; callers that need to
/// distinguish "no governance loaded" from "zero permissions" (several
/// gossip/control handlers bail out of the whole handler on the former)
/// keep the `Option`.
#[must_use]
pub fn permissions_for_pseudonym(
    cs: &crate::state::CommunityState,
    pseudonym_hex: &str,
    channel_id: Option<&rekindle_types::id::ChannelId>,
) -> Option<u64> {
    let gov = cs.governance_state.as_ref()?;
    let bytes = hex::decode(pseudonym_hex).ok()?;
    let pseudo_bytes: [u8; 32] = bytes.try_into().ok()?;
    let pseudo = rekindle_types::id::PseudonymKey(pseudo_bytes);
    let now = rekindle_utils::timestamp_secs();
    Some(rekindle_governance::permissions::compute_permissions(
        &pseudo, channel_id, gov, now,
    ))
}

/// Like [`permissions_for_pseudonym`], looking the community up by id
/// first. `None` when the community itself isn't known yet either.
#[must_use]
pub fn permissions_for(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_hex: &str,
    channel_id: Option<&rekindle_types::id::ChannelId>,
) -> Option<u64> {
    let communities = state.communities.read();
    let cs = communities.get(community_id)?;
    permissions_for_pseudonym(cs, pseudonym_hex, channel_id)
}

/// Compute effective permissions for the local user in a community channel.
/// Uses the cached CRDT governance state and returns 0 until governance loads.
pub fn my_permissions(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&rekindle_types::id::ChannelId>,
) -> u64 {
    let communities = state.communities.read();
    let Some(cs) = communities.get(community_id) else {
        return 0;
    };
    let Some(pseudo_hex) = cs.my_pseudonym_key.as_deref() else {
        return 0;
    };
    permissions_for_pseudonym(cs, pseudo_hex, channel_id).unwrap_or(0)
}
