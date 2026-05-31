//! Phase 21 REDO — thin facade.
//!
//! `write_our_presence` + `persist_discovered_registry_members` +
//! `decrypt_history_ranges` bodies live in
//! `rekindle_presence::community::{...}` parameterised over
//! `CommunityPresenceDeps`. `ensure_registry_open` stays src-tauri
//! because it mutates `community.open_community_records` (an
//! AppState-specific bookkeeping field) and the surrounding lock
//! ordering is intricate.
//!
//! Re-exports `DiscoveredRow` (the per-row scan tuple) for the
//! still-src-tauri-side `poll.rs::presence_poll_tick` orchestrator.
//! When 21.i-REDO ports `presence_poll_tick`, this file collapses
//! further (or disappears entirely).

use std::sync::Arc;

use rekindle_protocol::dht::DHTManager;

use crate::services::presence_adapter::build_adapter;
use crate::state::AppState;
use crate::state_helpers;

/// Per-row tuple yielded by the per-segment registry scan. Re-exported
/// from the crate so callers in the still-src-tauri-side
/// `poll.rs::presence_poll_tick` keep compiling.
pub(crate) use rekindle_presence::DiscoveredRow;
use rekindle_presence::CommunityPresenceDeps;

pub(crate) async fn ensure_registry_open(
    state: &Arc<AppState>,
    community_id: &str,
    mgr: &DHTManager,
    registry_key: &str,
) -> Result<(), String> {
    let records_open = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|c| c.open_community_records.records_open)
    };
    if records_open {
        return Ok(());
    }

    let (registry_kp, slot_kp) = {
        let communities = state.communities.read();
        let c = communities.get(community_id);
        (
            c.and_then(|c| c.registry_owner_keypair.clone()),
            c.and_then(|c| c.slot_keypair.clone()),
        )
    };
    let writer_kp = registry_kp.or(slot_kp);
    let opened = if let Some(ref kp_str) = writer_kp {
        if let Ok(kp) = kp_str.parse::<veilid_core::KeyPair>() {
            mgr.open_record_writable(registry_key, kp).await.is_ok()
        } else {
            false
        }
    } else {
        false
    };
    if !opened {
        mgr.open_record(registry_key)
            .await
            .map_err(|e| format!("presence_poll: failed to open registry: {e}"))?;
    }
    let registry_key_owned = registry_key.to_string();
    state_helpers::track_open_records(state, std::slice::from_ref(&registry_key_owned));
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.open_community_records.registry_key = Some(registry_key.to_string());
            cs.open_community_records.registry_writer = writer_kp;
            cs.open_community_records.records_open = true;
        }
    }
    tracing::debug!(community = %community_id, "presence_poll: re-opened registry after restart");
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
        community_id,
        &registry_key,
        &creds.my_pseudonym_hex,
        creds.my_subkey_index,
        creds.slot_keypair_str.as_deref(),
        creds.slot_seed_hex.is_some(),
        history_ranges,
    )
    .await;
}

pub(crate) fn persist_discovered_registry_members(
    state: &Arc<AppState>,
    community_id: &str,
    discovered_members: &[DiscoveredRow],
    member_roles: &std::collections::HashMap<String, Vec<u32>>,
    banned_members: &std::collections::HashSet<String>,
) {
    let Some(adapter) = build_adapter(state) else {
        tracing::debug!(
            community = %community_id,
            "persist_discovered_registry_members: adapter unavailable",
        );
        return;
    };
    rekindle_presence::persist_discovered_registry_members(
        &adapter,
        community_id,
        discovered_members,
        member_roles,
        banned_members,
    );
}

/// W11.2 — decrypt history ranges from a peer's presence row using
/// the MEK generation declared by the sender. Stays src-tauri because
/// the only caller is the still-src-tauri-side message-receive path
/// in `super::super`; lifting it would force a trivial trait method
/// for a 12-LoC body.
pub(in crate::services::community) fn decrypt_history_ranges(
    state: &Arc<AppState>,
    community_id: &str,
    encrypted: &rekindle_types::presence::EncryptedHistoryRanges,
) -> Option<Vec<rekindle_types::presence::HistoryRange>> {
    let mek = {
        let cache = state.mek_cache.lock();
        cache.get(community_id).cloned()?
    };
    if mek.generation() != encrypted.mek_generation {
        return None;
    }
    let plaintext = mek.decrypt(&encrypted.ciphertext).ok()?;
    serde_json::from_slice(&plaintext).ok()
}
