use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::member::MemberInfo;
use rekindle_types::message::BootstrapChannelMessages;
use tauri::Manager as _;

use crate::db::DbPool;
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
#[allow(
    dead_code,
    reason = "Phase 18 chiral split — non-`recent_messages` fields are kept for parity with the wire payload + future telemetry; only recent_messages is consumed by persist_bootstrap_recent_messages today"
)]
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
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rc.app_call(veilid_core::Target::RouteId(route_id), request_bytes),
    )
    .await
    .map_err(|_| "bootstrap app_call timed out".to_string())?
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
/// the channel MEK we just received, then upsert into the local
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
    let Some(app_handle) = state_helpers::app_handle(state) else {
        return;
    };
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return;
    };
    let mut decrypted: Vec<(String, String, String, String, i64)> = Vec::new();
    for channel in &bundle.recent_messages {
        let mek_pair = {
            let cache = state.channel_mek_cache.lock();
            cache
                .get(&(community_id.to_string(), channel.channel_id.clone()))
                .map(|mek| (*mek.as_bytes(), mek.generation()))
        };
        let Some((mek_bytes, mek_generation)) = mek_pair else {
            continue;
        };
        let mek = MediaEncryptionKey::from_bytes(mek_bytes, mek_generation);
        for entry in &channel.messages {
            if entry.mek_generation != mek_generation {
                continue;
            }
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
    let _ = db_call(pool.inner(), move |conn| {
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
