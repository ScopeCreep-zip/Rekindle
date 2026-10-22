//! Phase 21 REDO — thin facade.
//!
//! `write_our_presence` + `decrypt_history_ranges` bodies live in
//! `rekindle_presence::community::{...}` parameterised over
//! `CommunityPresenceDeps`. `ensure_registry_open` stays src-tauri
//! because it mutates `community.open_community_records` (an
//! AppState-specific bookkeeping field) and the surrounding lock
//! ordering is intricate.

use std::sync::Arc;

use crate::services::presence_adapter::build_adapter;
use crate::state::AppState;
use crate::state_helpers;

use rekindle_presence::CommunityPresenceDeps;

/// Ensure the community holds its member-registry record, **writable** when we
/// hold a writer keypair.
///
/// The community's registry lease (plan C7.5) is the answer: when held, the
/// record is open with its sticky writer and the pool re-arms its watch, so
/// there is nothing to do. When not (a failed open at hydration, or a
/// community joined this session before its hand-over), this borrows it and
/// hands the lease to the community, healing on the next poll tick.
pub(crate) async fn ensure_registry_open(
    state: &Arc<AppState>,
    community_id: &str,
    registry_key: &str,
) -> Result<(), String> {
    let (held, writer) = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        (
            c.leases.registry.is_some(),
            c.registry_owner_keypair.clone().or(c.slot_keypair.clone()),
        )
    };
    if held {
        return Ok(());
    }
    let lease = state_helpers::record_pool(state)?
        .acquire(registry_key, writer.as_deref())
        .await
        .map_err(|e| format!("presence_poll: failed to open registry: {e}"))?;
    if let Some(cs) = state.communities.write().get_mut(community_id) {
        cs.open_community_records.registry_key = Some(registry_key.to_string());
    }
    crate::services::community::leases::hold(
        state,
        community_id,
        rekindle_records::lease::CommunityLeases {
            registry: Some(lease),
            ..Default::default()
        },
    )
    .await;
    tracing::trace!(community = %community_id, "presence_poll: registry lease ensured");
    Ok(())
}

/// Publish a full `MemberPresence` row into the community's
/// registry-record subkey on demand (architecture §4.3). High-level
/// helper: looks up registry key + slot credentials + history
/// ranges + W26 signs + writes via the crate orchestrator.
///
/// Exposed as a public src-tauri surface for the
/// `update_community_presence` Tauri command — when the user
/// changes their status or game info, the gossip envelope reaches
/// online peers immediately AND this call refreshes the registry
/// row so peers reading the registry directly (returning members,
/// fresh joiners) see the new status without waiting for the next
/// presence-poll cadence tick. The lower-level
/// `rekindle_presence::write_our_presence(deps, …)` stays the
/// crate's authoritative builder; this facade does the AppState
/// lookups that the orchestrator does inline inside
/// `presence_poll_tick`.
pub async fn write_our_presence(state: &Arc<AppState>, community_id: &str) {
    let Some(adapter) = build_adapter(state) else {
        tracing::debug!(community = %community_id, "write_our_presence: adapter unavailable");
        return;
    };
    let Some(registry_key) = (match adapter.ensure_registry_open(community_id).await {
        Ok(key) => key,
        Err(error) => {
            tracing::warn!(community = %community_id, %error, "write_our_presence: ensure_registry_open failed");
            return;
        }
    }) else {
        tracing::debug!(community = %community_id, "write_our_presence: no registry key (community not joined yet)");
        return;
    };
    let Some(creds) = adapter.presence_credentials(community_id) else {
        tracing::debug!(community = %community_id, "write_our_presence: no presence credentials");
        return;
    };
    let history_ranges = adapter.compute_history_ranges(community_id).await;
    rekindle_presence::write_our_presence(
        &adapter,
        rekindle_presence::PresenceWrite {
            community_id,
            registry_key: &registry_key,
            my_pseudonym_hex: &creds.my_pseudonym_hex,
            my_subkey_index: creds.my_subkey_index,
            slot_keypair_str: creds.slot_keypair_str.as_deref(),
            has_slot_seed: creds.slot_seed_hex.is_some(),
            history_ranges,
        },
    )
    .await;
}
