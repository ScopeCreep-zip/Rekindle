//! Phase 17.f.2 — surviving helpers used by the still-src-tauri parts
//! of `mek_rotation.rs`. The cascade-election + distribute + recipient
//! enumeration logic moved into the `rekindle-mek-rotation` crate via
//! `MekDistributeDeps` (impl in `crate::services::mek_adapter`).
//!
//! What stays:
//! * `lookup_mek` — channel/community MEK cache lookup + keystore fallback
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
use rekindle_types::id::PseudonymKey;
use tauri::Manager;

use crate::channels::CommunityEvent;
use crate::state::AppState;

pub(crate) fn lookup_mek(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) -> Option<MediaEncryptionKey> {
    {
        let cache = state.channel_mek_cache.lock();
        if let Some(mek) = cache.get(&(community_id.to_string(), channel_id.to_string())) {
            if mek.generation() == generation {
                return Some(mek.clone());
            }
        }
    }
    {
        let cache = state.mek_cache.lock();
        if let Some(mek) = cache.get(community_id) {
            if mek.generation() == generation {
                return Some(mek.clone());
            }
        }
    }

    let keystore: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let guard = keystore.lock();
    let ks = guard.as_ref()?;
    crate::keystore::load_channel_mek_generation(ks, community_id, channel_id, generation).or_else(
        || crate::keystore::load_mek(ks, community_id).filter(|mek| mek.generation() == generation),
    )
}

pub(crate) fn update_generation_state(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
    generation: u64,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.mek_generation = generation;
        if let Some(channel_id) = channel_id {
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
    channel_id: Option<&str>,
    generation: u64,
) {
    crate::event_dispatch::emit_live(
        app_handle,
        "community-event",
        &CommunityEvent::MekRotated {
            community_id: community_id.to_string(),
            channel_id: channel_id.map(ToOwned::to_owned),
            new_generation: generation,
        },
    );
}

pub(crate) fn persist_mek(
    app_handle: &tauri::AppHandle,
    community_id: &str,
    channel_id: Option<&str>,
    mek: &MediaEncryptionKey,
) {
    let keystore: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let guard = keystore.lock();
    if let Some(ks) = guard.as_ref() {
        crate::keystore::store_mek(ks, community_id, channel_id, mek);
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
