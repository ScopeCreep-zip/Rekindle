use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::channels::CommunityEvent;
use crate::db::DbPool;
use crate::services::message_service;
use crate::state::{AppState, OnlineMember};
use crate::state_helpers;

use rekindle_protocol::dht::community::envelope::{
    verify_envelope, CommunityEnvelope, ControlPayload, SignedEnvelope,
};

pub async fn handle(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    msg: veilid_core::VeilidAppMessage,
) {
    let message = msg.message().to_vec();
    tracing::info!(msg_len = message.len(), "app_message received");

    if !message.is_empty() && message[0] == b'V' {
        let voice_data = &message[1..];
        match rekindle_voice::transport::VoiceTransport::receive(voice_data) {
            Ok(packet) => {
                let tx = state.voice_packet_tx.read().clone();
                if let Some(tx) = tx {
                    if tx.try_send(packet).is_err() {
                        tracing::trace!("voice packet channel full or closed, dropping packet");
                    }
                }
            }
            Err(e) => {
                tracing::trace!(error = %e, "failed to deserialize voice packet");
            }
        }
        return;
    }

    if let Ok(signed) = serde_json::from_slice::<SignedEnvelope>(&message) {
        handle_gossip_envelope(app_handle, state, signed).await;
        return;
    }

    let pool: tauri::State<'_, DbPool> = app_handle.state();
    message_service::handle_incoming_message(app_handle, state, pool.inner(), &message).await;
}

async fn handle_gossip_envelope(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    signed: SignedEnvelope,
) {
    let community_id = &signed.community_id;

    let dedup_key = extract_dedup_key(&signed);
    {
        let mut cache = state.dedup_cache.lock();
        if cache.check_and_insert(community_id, &signed.sender_pseudonym, &dedup_key) {
            tracing::trace!(dedup_key = %dedup_key, "gossip dedup: dropping duplicate");
            return;
        }
    }

    if let Err(e) = verify_envelope(&signed) {
        tracing::warn!(error = %e, "rejecting gossip envelope: bad signature");
        return;
    }

    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.known_members.insert(signed.sender_pseudonym.clone());

            if let Some(ref mut gossip) = cs.gossip {
                if let Some(member) = gossip.online_members.get_mut(&signed.sender_pseudonym) {
                    member.last_seen = rekindle_utils::timestamp_secs();
                }
                if let Some(member) = gossip.peers.get_mut(&signed.sender_pseudonym) {
                    member.last_seen = rekindle_utils::timestamp_secs();
                }
            }
        }
    }

    let is_private = is_private_control_payload(&signed.envelope_bytes);
    if signed.ttl > 0 && !is_private {
        gossip_forward(state, community_id, &signed);
    }

    let is_from_self = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.as_ref())
            .is_some_and(|pk| pk == &signed.sender_pseudonym)
    };
    if !is_from_self {
        handle_relayed_envelope(app_handle, state, signed).await;
    }
}

fn extract_dedup_key(signed: &SignedEnvelope) -> String {
    if let Ok(env) = serde_json::from_slice::<CommunityEnvelope>(&signed.envelope_bytes) {
        match env {
            CommunityEnvelope::MessageNotification { ref message_id, .. } => message_id.clone(),
            CommunityEnvelope::TypingIndicator {
                ref channel_id,
                ref pseudonym_key,
            } => {
                let bucket = rekindle_utils::timestamp_secs() / 5;
                format!("typing:{channel_id}:{pseudonym_key}:{bucket}")
            }
            CommunityEnvelope::PresenceUpdate {
                ref pseudonym_key, ..
            } => {
                let bucket = rekindle_utils::timestamp_secs() / 30;
                format!("presence:{pseudonym_key}:{bucket}")
            }
            CommunityEnvelope::Control(_) => envelope_hash(&signed.envelope_bytes),
        }
    } else {
        envelope_hash(&signed.envelope_bytes)
    }
}

fn envelope_hash(envelope_bytes: &[u8]) -> String {
    use blake2::{digest::consts::U16, Blake2b, Digest};

    let mut h = Blake2b::<U16>::new();
    h.update(envelope_bytes);
    hex::encode(h.finalize())
}

pub(crate) fn is_private_control_payload(envelope_bytes: &[u8]) -> bool {
    if let Ok(CommunityEnvelope::Control(ref payload)) =
        serde_json::from_slice::<CommunityEnvelope>(envelope_bytes)
    {
        matches!(
            payload,
            ControlPayload::JoinAccepted { .. }
                | ControlPayload::SlotKeypairGrant { .. }
                | ControlPayload::AdminKeypairGrant { .. }
                | ControlPayload::SyncResponse { .. }
        )
    } else {
        false
    }
}

fn gossip_forward(state: &Arc<AppState>, community_id: &str, signed: &SignedEnvelope) {
    let mut forward = signed.clone();
    forward.ttl = forward.ttl.saturating_sub(1);
    let Ok(signed_bytes) = serde_json::to_vec(&forward) else {
        return;
    };

    let Some(rc) = state_helpers::safe_routing_context(state) else {
        return;
    };

    let peers: Vec<Vec<u8>> = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else {
            return;
        };
        let Some(ref gossip) = cs.gossip else {
            return;
        };
        gossip
            .peers
            .iter()
            .filter(|(pk, _)| *pk != &signed.sender_pseudonym)
            .map(|(_, m)| m.route_blob.clone())
            .collect()
    };

    if peers.is_empty() {
        return;
    }

    for route_blob in peers {
        let rc = rc.clone();
        let data = signed_bytes.clone();
        tokio::spawn(async move {
            match rc.api().import_remote_private_route(route_blob) {
                Ok(route_id) => {
                    let _ = rc
                        .app_message(veilid_core::Target::RouteId(route_id), data)
                        .await;
                }
                Err(e) => {
                    tracing::trace!(error = %e, "gossip forward: route import failed");
                }
            }
        });
    }
}

async fn handle_relayed_envelope(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    signed: SignedEnvelope,
) {
    let envelope: CommunityEnvelope = match serde_json::from_slice(&signed.envelope_bytes) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "invalid relayed envelope");
            return;
        }
    };

    let community_id = signed.community_id.clone();

    match envelope {
        CommunityEnvelope::MessageNotification {
            channel_id,
            message_id,
            sequence,
            subkey_index,
            content_hash,
            ..
        } => {
            let pending = crate::services::community::message_notifications::PendingMessageFetch {
                community_id: community_id.clone(),
                channel_id,
                message_id,
                subkey_index,
                sequence,
                content_hash,
                attempt: 0,
            };
            if let Err(e) =
                crate::services::community::handle_message_notification(app_handle, state, pending)
                    .await
            {
                tracing::debug!(
                    community = %community_id,
                    error = %e,
                    "message notification handling failed"
                );
            }
        }
        CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ChannelTyping {
                    community_id,
                    channel_id,
                    pseudonym_key,
                },
            );
        }
        CommunityEnvelope::PresenceUpdate {
            pseudonym_key,
            status,
            game_info,
            route_blob,
        } => {
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(&community_id) {
                    cs.known_members.insert(pseudonym_key.clone());
                    if let Some(ref mut gossip) = cs.gossip {
                        if status == "offline" {
                            gossip.online_members.remove(&pseudonym_key);
                            gossip.peers.remove(&pseudonym_key);
                        } else if let Some(ref blob) = route_blob {
                            if !blob.is_empty() {
                                let member = OnlineMember {
                                    route_blob: blob.clone(),
                                    status: status.clone(),
                                    last_seen: rekindle_utils::timestamp_secs(),
                                };
                                gossip
                                    .online_members
                                    .insert(pseudonym_key.clone(), member.clone());
                                gossip.peers.insert(pseudonym_key.clone(), member);
                            }
                        }
                    }
                }
            }

            let (game_name, game_id, elapsed_seconds, server_address) = if let Some(gi) = game_info
            {
                (
                    Some(gi.game_name),
                    gi.game_id,
                    gi.elapsed_seconds,
                    gi.server_address,
                )
            } else {
                (None, None, None, None)
            };

            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberPresenceChanged {
                    community_id,
                    pseudonym_key,
                    status,
                    game_name,
                    game_id,
                    elapsed_seconds,
                    server_address,
                },
            );
        }
        CommunityEnvelope::Control(payload) => {
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            super::control::handle_relayed_control(
                app_handle,
                state,
                &pool,
                &community_id,
                &signed.sender_pseudonym,
                payload,
            )
            .await;
        }
    }
}
