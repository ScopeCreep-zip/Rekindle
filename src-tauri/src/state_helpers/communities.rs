//! Community channel-list write helpers + governance-key collection.

use std::sync::Arc;

use crate::state::AppState;

/// Replace the entire channel list for a community.
pub fn set_community_channels(
    state: &Arc<AppState>,
    community_id: &str,
    channels: Vec<crate::state::ChannelInfo>,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.channels = channels;
    }
}

/// Append a single channel to a community's channel list.
pub fn push_community_channel(
    state: &Arc<AppState>,
    community_id: &str,
    channel: crate::state::ChannelInfo,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.channels.push(channel);
    }
}

/// Our pseudonym public key (hex) in a community, if joined and primed.
///
/// THE accessor for the `communities.read().get(id).and_then(|c|
/// c.my_pseudonym_key.clone())` pattern that was inlined across
/// adapters and runtimes — delegate here instead of re-spelling it.
pub fn my_pseudonym_key(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|c| c.my_pseudonym_key.clone())
}

/// Collect communities with governance record keys.
pub fn communities_with_governance_keys(state: &Arc<AppState>) -> Vec<(String, String)> {
    state
        .communities
        .read()
        .values()
        .filter_map(|c| c.governance_key.as_ref().map(|k| (c.id.clone(), k.clone())))
        .collect()
}

/// The MEK hierarchy for CHANNEL MEDIA (voice frames, video frames):
/// the per-channel MEK when the §10.5 join/leave rotation has
/// distributed one, otherwise the community MEK every member holds
/// from join. This mirrors the text plane's
/// `channel_or_community_mek_impl` — one key hierarchy for every
/// channel payload. Stage channels never rotate (§10.7), so they
/// resolve to the community MEK by construction. Returns
/// `(key_bytes, generation)`.
pub fn channel_media_mek(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<([u8; 32], u64)> {
    let channel = state
        .channel_mek_cache
        .lock()
        .get(&(community_id.to_string(), channel_id.to_string()))
        .map(|m| (*m.as_bytes(), m.generation()));
    channel.or_else(|| {
        state
            .mek_cache
            .lock()
            .get(community_id)
            .map(|m| (*m.as_bytes(), m.generation()))
    })
}

/// Like [`channel_media_mek`] but returns the FULL cached key (clone),
/// preserving its provenance (rotator pseudonym + election rank). Use this
/// when the key will be re-distributed (e.g. answering a `RequestMEK`) so the
/// recipient learns the canonical rank and converges correctly — reconstructing
/// via `from_bytes` would strip provenance and could let a requester later flip
/// to a non-canonical same-generation key.
pub fn channel_media_mek_full(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    let channel = state
        .channel_mek_cache
        .lock()
        .get(&(community_id.to_string(), channel_id.to_string()))
        .cloned();
    channel.or_else(|| state.mek_cache.lock().get(community_id).cloned())
}

/// Retention window for the REPLACED channel key after a rotation —
/// in-flight media encrypted under the old generation still decrypts
/// during the transition. Matches Discord DAVE's previous-epoch
/// ratchet retention ("up to ten seconds") and SFrame RFC 9605's
/// "old key may be kept for some time ... deleted promptly".
pub const PREV_MEK_RETENTION: std::time::Duration = std::time::Duration::from_secs(10);

/// The ONLY way a channel MEK enters the live cache. Refuses
/// downgrades (an older-generation transfer must never replace the
/// live key — rollback vector) and parks the key it replaces in the
/// previous-generation slot for [`PREV_MEK_RETENTION`]. Returns
/// `false` when the install was refused.
pub fn install_channel_mek(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    mek: rekindle_crypto::group::media_key::MediaEncryptionKey,
) -> bool {
    let key = (community_id.to_string(), channel_id.to_string());
    let mut cache = state.channel_mek_cache.lock();
    if let Some(cached) = cache.get(&key) {
        if cached.generation() > mek.generation() {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                incoming = mek.generation(),
                cached = cached.generation(),
                "channel MEK older than cached — not applied to live cache"
            );
            return false;
        }
        if cached.generation() == mek.generation() {
            // Same-generation collision (split-brain): two rotators minted
            // different random keys for this generation. Keep the canonical
            // one — the key whose minter has the lowest deterministic election
            // rank (= the rightful primary rotator). Every peer applies this
            // pure comparison and converges on the identical key, regardless of
            // which transfer arrived first.
            if !rekindle_mek_rotation::convergence::incoming_wins_same_generation(
                cached.election_rank().as_ref(),
                mek.election_rank().as_ref(),
            ) {
                return false;
            }
            // Incoming is more canonical — park the superseded same-gen key so
            // in-flight packets encrypted under it still decrypt during the
            // brief convergence window (PREV_MEK_RETENTION).
            state
                .channel_mek_prev
                .lock()
                .insert(key.clone(), (cached.clone(), std::time::Instant::now()));
        } else if cached.generation() < mek.generation() {
            state
                .channel_mek_prev
                .lock()
                .insert(key.clone(), (cached.clone(), std::time::Instant::now()));
        }
    }
    cache.insert(key, mek);
    true
}

/// The ONLY way a community-wide MEK enters the live cache. The
/// community analogue of [`install_channel_mek`]: refuses downgrades (an
/// older-generation transfer must never replace the live key — rollback
/// vector) and resolves same-generation split-brain by keeping the key
/// whose minter has the lowest deterministic election rank, so every peer
/// converges on the identical key regardless of arrival order. Returns
/// `false` when the install was refused.
///
/// Every write to `state.mek_cache` MUST go through here (rotation, received
/// transfer, governance hydration, restore) so no path can re-introduce the
/// last-write-wins divergence. (The community cache has no previous-key
/// window like channels do; community-wide MEK rotations are rare and the
/// 1:1 retention need is covered at the channel layer.)
pub fn install_community_mek(
    state: &Arc<AppState>,
    community_id: &str,
    mek: rekindle_crypto::group::media_key::MediaEncryptionKey,
) -> bool {
    let mut cache = state.mek_cache.lock();
    if let Some(cached) = cache.get(community_id) {
        if cached.generation() > mek.generation() {
            tracing::debug!(
                community = %community_id,
                incoming = mek.generation(),
                cached = cached.generation(),
                "community MEK older than cached — not applied to live cache"
            );
            return false;
        }
        if cached.generation() == mek.generation()
            && !rekindle_mek_rotation::convergence::incoming_wins_same_generation(
                cached.election_rank().as_ref(),
                mek.election_rank().as_ref(),
            )
        {
            return false;
        }
    }
    cache.insert(community_id.to_string(), mek);
    true
}

/// The replaced channel key, while still inside [`PREV_MEK_RETENTION`]
/// — receive paths consult this on a generation mismatch so in-flight
/// old-generation media decrypts instead of freezing every rotation.
/// Expired entries are pruned on read.
pub fn previous_channel_mek(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<([u8; 32], u64)> {
    let key = (community_id.to_string(), channel_id.to_string());
    let mut prev = state.channel_mek_prev.lock();
    match prev.get(&key) {
        Some((_, installed)) if installed.elapsed() > PREV_MEK_RETENTION => {
            prev.remove(&key);
            None
        }
        Some((mek, _)) => Some((*mek.as_bytes(), mek.generation())),
        None => None,
    }
}
