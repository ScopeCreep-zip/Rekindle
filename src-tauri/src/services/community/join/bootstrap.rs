use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::member::MemberInfo;
use rekindle_types::message::BootstrapChannelMessages;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

/// Member-list entry shape consumed by the join flow. Kept as a thin
/// alias over `rekindle_types::member::MemberInfo` so the callers below
/// don't need to change beyond their field accesses.
pub(super) type BootstrapMemberEntry = MemberInfo;

/// One channel's worth of recent messages from §14.4 BootstrapResponse —
/// re-export of the typed wire shape from `rekindle-types`.
pub(super) type BootstrapRecentChannel = BootstrapChannelMessages;

#[derive(Debug, Clone)]
pub(super) struct BootstrapBundle {
    pub member_list: Vec<BootstrapMemberEntry>,
    pub governance_entry_count: usize,
    pub channel_mek_count: usize,
    pub recent_messages: Vec<BootstrapRecentChannel>,
    pub has_wrapped_owner_keypair: bool,
}

pub(super) async fn fetch_bootstrap_bundle(
    state: &Arc<AppState>,
    governance_key: &str,
    inviter_route_blob: &[u8],
    joiner_pseudonym: &str,
) -> Result<BootstrapBundle, String> {
    let route_id = state_helpers::import_route_blob(state, inviter_route_blob)?;
    let rc = state_helpers::safe_routing_context(state).ok_or("Veilid node not attached")?;
    let request = CommunityEnvelope::Control(ControlPayload::BootstrapRequest {
        joiner_pseudonym: joiner_pseudonym.to_string(),
        governance_key: governance_key.to_string(),
    });
    let request_bytes = rekindle_protocol::capnp_envelope::encode_community_envelope(&request)
        .map_err(|e| format!("encode bootstrap request: {e}"))?;
    // Bounded by Veilid's own reply timeout for the route (plan C4.L1b).
    let response = rc
        .app_call(veilid_core::Target::RouteId(route_id), request_bytes)
        .await
        .map_err(|e| format!("bootstrap app_call failed: {e}"))?;

    match rekindle_protocol::capnp_envelope::decode_community_envelope(&response)
        .map_err(|e| format!("invalid bootstrap response envelope: {e}"))?
    {
        CommunityEnvelope::Control(ControlPayload::BootstrapResponse {
            governance_entries,
            member_list,
            channel_meks,
            recent_messages,
            wrapped_owner_keypair,
        }) => Ok(BootstrapBundle {
            member_list,
            governance_entry_count: governance_entries.len(),
            channel_mek_count: channel_meks.len(),
            recent_messages,
            has_wrapped_owner_keypair: !wrapped_owner_keypair.is_empty(),
        }),
        _ => Err("unexpected bootstrap response payload".into()),
    }
}

/// Decrypt every message in the bundle's `recent_messages` block under
/// its channel's text key at the generation it names, then upsert into the local
/// messages table. Called once at the end of the join flow so the
/// joiner has scrollback without waiting for the history-ad path.
pub(super) async fn persist_bootstrap_recent_messages(
    state: &Arc<AppState>,
    community_id: &str,
    bundle: &BootstrapBundle,
) {
    if bundle.recent_messages.is_empty() {
        return;
    }
    let Ok(pool) = state.db.current() else {
        tracing::debug!("bootstrap persist: no identity database — dropped");
        return;
    };
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return;
    };
    let mut decrypted: Vec<(String, String, String, String, i64)> = Vec::new();
    // Each bundled message is sealed under exactly the generation of its
    // channel's text key it names (the responder's `build_channel_envelope`).
    let keys = state_helpers::key_provider(state);
    for channel in &bundle.recent_messages {
        let Some(channel_id) = rekindle_types::id::ChannelId::from_hex(&channel.channel_id) else {
            continue;
        };
        let scope = keys.scope_for_text(community_id, channel_id);
        for entry in &channel.messages {
            let Some(key) = keys.key(
                community_id,
                scope,
                rekindle_types::channel_keys::KeyEpoch(entry.mek_generation),
            ) else {
                continue;
            };
            let mek = MediaEncryptionKey::from_bytes(*key, entry.mek_generation);
            let Ok(plaintext) = mek.decrypt(&entry.ciphertext) else {
                continue;
            };
            let Ok(body) = String::from_utf8(plaintext) else {
                continue;
            };
            decrypted.push((
                channel.channel_id.clone(),
                entry.message_id.clone(),
                entry.sender_pseudonym.clone(),
                body,
                entry.timestamp,
            ));
        }
    }
    if decrypted.is_empty() {
        return;
    }
    let owner = owner_key;
    let community = community_id.to_string();
    let _ = db_call(&pool, move |conn| {
        let tx = conn.transaction()?;
        for (channel_id, message_id, sender, body, ts) in &decrypted {
            tx.execute(
                "INSERT OR IGNORE INTO messages
                    (owner_key, community_id, conversation_id, conversation_type,
                     sender_key, body, timestamp, message_id)
                 VALUES (?1, ?2, ?3, 'channel', ?4, ?5, ?6, ?7)",
                rusqlite::params![owner, community, channel_id, sender, body, ts, message_id],
            )?;
        }
        tx.commit()
    })
    .await;
}

/// Seed the local member roster from §14.4 `BootstrapResponse.member_list`
/// so the joiner sees the full community membership immediately, rather
/// than waiting for the DHT inspect/watch loop to rediscover each slot.
/// `INSERT OR IGNORE` keeps any richer row a concurrent registry poll may
/// have already written; the per-community profile columns are filled from
/// the bootstrap snapshot when present.
pub(super) async fn persist_bootstrap_members(
    state: &Arc<AppState>,
    community_id: &str,
    bundle: &BootstrapBundle,
) {
    if bundle.member_list.is_empty() {
        return;
    }
    let Ok(pool) = state.db.current() else {
        tracing::debug!("bootstrap persist: no identity database — dropped");
        return;
    };
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return;
    };
    let now = rekindle_utils::timestamp_secs().cast_signed();
    let community = community_id.to_string();
    let members = bundle.member_list.clone();
    if let Err(e) = db_call(&pool, move |conn| {
        let tx = conn.transaction()?;
        for m in &members {
            rekindle_db::repo::members::insert_bootstrap_if_absent(
                &tx, &owner_key, &community, m, now,
            )?;
        }
        tx.commit()
    })
    .await
    {
        tracing::warn!(community = %community_id, error = %e, "bootstrap member list not persisted");
    }
}
