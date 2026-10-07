//! Phase 17.f.2 — surviving helpers used by the still-src-tauri parts
//! of `mek_rotation.rs`. The cascade-election + distribute + recipient
//! enumeration logic moved into the `rekindle-mek-rotation` crate via
//! `MekDistributeDeps` (impl in `crate::services::mek_adapter`).
//!
//! What stays:
//! * `lookup_mek` — exact-scope, exact-generation key lookup (cache, then keystore history)
//! * `current_generation` — read the current MEK generation from cache/state
//! * `update_generation_state` — mirror the cache write into AppState.communities
//! * `persist_mek` — Stronghold persistence wrapper
//! * `emit_rotation_event` — Tauri MekRotated emit
//! * `my_pseudonym_hex` / `my_pseudonym` / `pseudonym_from_hex`
//!
//! What moved into the crate (and was deleted from this file):
//! * `RotationRecipient`, `online_recipients`, `voice_recipients`,
//!   `effective_voice_participants` — `MekDistributeDeps` methods
//!   (with the same body) on `MekAdapter`.
//! * `cascade_delay`, `wait_for_rotation_slot`, `distribute_mek`,
//!   `generation_advanced`, `max_cascades`, `CASCADE_TIMEOUT_SECS`,
//!   `MAX_CASCADES` — `rekindle_mek_rotation::{cascade_delay,
//!   wait_for_rotation_slot, distribute_mek, MAX_CASCADES}`.

use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::PseudonymKey;

use crate::state::AppState;

/// The scope's full key (with provenance) at exactly `generation`: the
/// live keys, then the vault's history for that same scope. Never another
/// scope's key. Used to re-serve a key to a requester, where provenance
/// must travel with it.
pub(crate) fn lookup_mek(
    state: &Arc<AppState>,
    community_id: &str,
    scope: KeyScope,
    generation: u64,
) -> Option<MediaEncryptionKey> {
    use rekindle_mek_rotation::ChannelMekCache as _;
    if let Some(mek) = crate::state_helpers::LiveMekCache::new(Arc::clone(state)).get(
        community_id,
        scope,
        generation,
    ) {
        return Some(mek);
    }
    let guard = state.keystore.lock();
    crate::keystore::load_mek_generation(guard.as_ref()?, community_id, scope, generation)
}

/// Mirror a key install into the scope's generation field: the
/// community's for the community key, the channel's for a channel key.
pub(crate) fn update_generation_state(
    state: &Arc<AppState>,
    community_id: &str,
    scope: KeyScope,
    generation: u64,
) {
    let mut communities = state.communities.write();
    let Some(community) = communities.get_mut(community_id) else {
        return;
    };
    match scope {
        KeyScope::Community => community.mek_generation = generation,
        KeyScope::Channel(channel) => {
            let channel_id = channel.to_hex();
            if let Some(channel) = community
                .channels
                .iter_mut()
                .find(|channel| channel.id == channel_id)
            {
                channel.mek_generation = generation;
            }
        }
    }
}

pub(crate) fn emit_rotation_event(
    app_handle: &tauri::AppHandle,
    community_id: &str,
    scope: KeyScope,
    generation: u64,
) {
    crate::event_dispatch::emit_subscription(
        app_handle,
        &rekindle_types::subscription_events::SubscriptionEvent::Crypto(
            rekindle_types::subscription_events::CryptoEvent::MekRotated {
                community: community_id.to_string(),
                channel: scope.wire_channel(),
                generation,
                rotator_pseudonym: None,
            },
        ),
    );
}

pub(crate) fn persist_mek(
    state: &Arc<AppState>,
    community_id: &str,
    scope: KeyScope,
    mek: &MediaEncryptionKey,
) {
    let guard = state.keystore.lock();
    let Some(ks) = guard.as_ref() else {
        tracing::warn!(community = %community_id, %scope, "keystore locked — MEK not persisted");
        return;
    };
    if let Err(e) = crate::keystore::persist_mek(ks, community_id, scope, mek) {
        tracing::warn!(community = %community_id, %scope, error = %e, "MEK not persisted");
    }
}

pub(crate) fn my_pseudonym_hex(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|community| community.my_pseudonym_key.clone())
}

pub(crate) fn my_pseudonym(state: &Arc<AppState>, community_id: &str) -> Option<PseudonymKey> {
    my_pseudonym_hex(state, community_id)
        .as_deref()
        .and_then(pseudonym_from_hex)
}

pub(crate) fn pseudonym_from_hex(hex_str: &str) -> Option<PseudonymKey> {
    let bytes = hex::decode(hex_str).ok()?;
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    Some(PseudonymKey(bytes))
}
