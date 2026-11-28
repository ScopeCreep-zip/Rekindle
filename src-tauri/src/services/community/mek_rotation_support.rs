//! Phase 17.f.2 — surviving helpers used by the still-src-tauri parts
//! of `mek_rotation.rs`. The cascade-election + distribute + recipient
//! enumeration logic moved into the `rekindle-mek-rotation` crate via
//! `MekDistributeDeps` (impl in `crate::services::mek_adapter`).
//!
//! What stays:
//! * `current_generation` — read the current MEK generation from cache/state
//! * `update_generation_state` — mirror the cache write into AppState.communities
//! * `persist_mek` — Stronghold persistence wrapper
//! * `emit_rotation_event` — Tauri MekRotated emit
//!
//! Deleted with the last-resort mint's election (FX3), their only user:
//! `my_pseudonym_hex`, `my_pseudonym`, `pseudonym_from_hex`.
//!
//! What moved into the crate (and was deleted from this file):
//! * `RotationRecipient`, `online_recipients` — `MekDistributeDeps` methods
//!   (with the same body) on `MekAdapter`.
//! * `cascade_delay`, `wait_for_rotation_slot`, `distribute_mek`,
//!   `generation_advanced`, `max_cascades`, `CASCADE_TIMEOUT_SECS`,
//!   `MAX_CASCADES` — `rekindle_mek_rotation::{cascade_delay,
//!   wait_for_rotation_slot, distribute_mek, MAX_CASCADES}`.

use std::sync::Arc;

use rekindle_types::channel_keys::KeyScope;

use crate::state::AppState;

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
