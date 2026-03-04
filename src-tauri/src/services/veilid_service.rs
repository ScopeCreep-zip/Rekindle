use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

use crate::channels::{NetworkStatusEvent, NotificationEvent};
use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::{AppState, DHTManagerHandle, NodeHandle, RoutingManagerHandle};
use crate::state_helpers;

/// Build and emit a `NetworkStatusEvent` from current `NodeHandle` state.
///
/// Called from any code path that changes attachment, readiness, or route status
/// so the frontend's `NetworkIndicator` updates instantly.
pub fn emit_network_status(app_handle: &tauri::AppHandle, state: &AppState) {
    let event = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) => NetworkStatusEvent {
                attachment_state: nh.attachment_state.clone(),
                is_attached: nh.is_attached,
                public_internet_ready: nh.public_internet_ready,
                has_route: nh.route_blob.is_some(),
            },
            None => NetworkStatusEvent {
                attachment_state: "detached".to_string(),
                is_attached: false,
                public_internet_ready: false,
                has_route: false,
            },
        }
    };
    let _ = app_handle.emit("network-status", &event);
}

/// Start the Veilid event dispatch loop.
///
/// This is the heartbeat of the application. It receives real `VeilidUpdate`
/// events from the node's internal callback channel and routes them to
/// the appropriate service handler.
pub async fn start_dispatch_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("veilid dispatch loop started");

    loop {
        tokio::select! {
            Some(update) = update_rx.recv() => {
                handle_veilid_update(&app_handle, &state, update).await;
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("veilid dispatch loop shutting down");
                break;
            }
        }
    }
}

/// Route a single `VeilidUpdate` to the appropriate handler.
async fn handle_veilid_update(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    update: VeilidUpdate,
) {
    match update {
        VeilidUpdate::AppMessage(msg) => handle_app_message(app_handle, state, *msg).await,
        VeilidUpdate::AppCall(call) => handle_app_call(app_handle, state, *call).await,
        VeilidUpdate::ValueChange(change) => {
            handle_value_change(app_handle, state, *change).await;
        }
        VeilidUpdate::Attachment(attachment) => {
            handle_attachment(app_handle, state, &attachment);
        }
        VeilidUpdate::RouteChange(change) => {
            handle_route_change(app_handle, state, &change).await;
        }
        VeilidUpdate::Shutdown => {
            tracing::info!("veilid core shutdown event received");
        }
        // Log, Network, Config updates are informational
        _ => {}
    }
}

/// Handle an incoming `AppMessage` by routing it through the message service.
///
/// Routing order:
/// 1. Voice packets (prefixed with `b'V'`) → voice engine receive channel
/// 2. Community broadcasts (JSON) → community handler
/// 3. Everything else → standard message envelope handling
async fn handle_app_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    msg: veilid_core::VeilidAppMessage,
) {
    let message = msg.message().to_vec();
    tracing::debug!(msg_len = message.len(), "app_message received");

    // 1. Check for voice packet (tagged with b'V' prefix)
    if !message.is_empty() && message[0] == b'V' {
        let voice_data = &message[1..];
        match rekindle_voice::transport::VoiceTransport::receive(voice_data) {
            Ok(packet) => {
                let tx = state.voice_packet_tx.read().clone();
                if let Some(tx) = tx {
                    if tx.try_send(packet).is_err() {
                        tracing::trace!("voice packet channel full or closed — dropping packet");
                    }
                }
            }
            Err(e) => {
                tracing::trace!(error = %e, "failed to deserialize voice packet");
            }
        }
        return;
    }

    // 2. Try to parse as a community SignedEnvelope (gossip mesh)
    if let Ok(signed) =
        serde_json::from_slice::<rekindle_protocol::dht::community::envelope::SignedEnvelope>(
            &message,
        )
    {
        handle_gossip_envelope(app_handle, state, signed).await;
        return;
    }

    // 3. Fallback to standard message handling
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    super::message_service::handle_incoming_message(app_handle, state, pool.inner(), &message)
        .await;
}

/// Handle a community envelope received via the gossip mesh.
///
/// Core of the gossip pipeline:
/// 1. Dedup check (drop if already seen)
/// 2. Verify Ed25519 signature
/// 3. If we're coordinator and it's a control payload, handle it
/// 4. Forward to D gossip peers (TTL permitting)
/// 5. Process locally (emit to frontend)
async fn handle_gossip_envelope(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    signed: rekindle_protocol::dht::community::envelope::SignedEnvelope,
) {
    use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

    let community_id = &signed.community_id;

    // 1. DEDUP CHECK — drop if we've already seen this message
    let dedup_key = extract_dedup_key(&signed);
    {
        let mut cache = state.dedup_cache.lock();
        if cache.check_and_insert(community_id, &signed.sender_pseudonym, &dedup_key) {
            tracing::trace!(dedup_key = %dedup_key, "gossip dedup: dropping duplicate");
            return;
        }
    }

    // 2. VERIFY SIGNATURE
    if let Err(e) = rekindle_protocol::dht::community::envelope::verify_envelope(&signed) {
        tracing::warn!(error = %e, "rejecting gossip envelope: bad signature");
        return;
    }

    // 3. COORDINATOR DISPATCH — if Control payload and we're coordinator, handle it
    let is_control = serde_json::from_slice::<CommunityEnvelope>(&signed.envelope_bytes)
        .map(|e| matches!(e, CommunityEnvelope::Control(_)))
        .unwrap_or(false);

    if is_control {
        let state_mgr = {
            let services = state.coordinator_services.read();
            services
                .get(community_id)
                .filter(|h| h.is_coordinator())
                .map(|h| h.state_mgr.clone())
        };
        if let Some(ref sm) = state_mgr {
            super::coordinator::state_manager::handle_incoming_envelope(
                state, sm, signed.clone(),
            )
            .await;
        }
    }

    // 4. FORWARD to D gossip peers (if TTL > 0)
    if signed.ttl > 0 {
        gossip_forward(state, community_id, &signed);
    }

    // 5. PROCESS LOCALLY (emit to frontend) — skip if we sent it
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

/// Extract a dedup key from a signed envelope.
///
/// Different envelope types use different dedup strategies:
/// - ChatMessage: use `message_id` (globally unique)
/// - TypingIndicator: bucket by 5-second window (ephemeral, don't need exact dedup)
/// - PresenceUpdate: bucket by 30-second window
/// - Control: BLAKE2b hash of envelope bytes (unique per payload)
fn extract_dedup_key(
    signed: &rekindle_protocol::dht::community::envelope::SignedEnvelope,
) -> String {
    use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

    if let Ok(env) = serde_json::from_slice::<CommunityEnvelope>(&signed.envelope_bytes) {
        match env {
            CommunityEnvelope::ChatMessage { ref message_id, .. } => message_id.clone(),
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
            CommunityEnvelope::Control(_) => {
                use blake2::{Blake2b, Digest, digest::consts::U16};
                let mut h = Blake2b::<U16>::new();
                h.update(&signed.envelope_bytes);
                hex::encode(h.finalize())
            }
        }
    } else {
        // Fallback: hash envelope bytes
        use blake2::{Blake2b, Digest, digest::consts::U16};
        let mut h = Blake2b::<U16>::new();
        h.update(&signed.envelope_bytes);
        hex::encode(h.finalize())
    }
}

/// Forward a received gossip envelope to our D gossip peers (excluding the sender).
///
/// Decrements TTL before forwarding. Each peer gets the signed bytes via `app_message`.
fn gossip_forward(
    state: &Arc<AppState>,
    community_id: &str,
    signed: &rekindle_protocol::dht::community::envelope::SignedEnvelope,
) {
    // Decrement TTL for forwarded copy
    let mut forward = signed.clone();
    forward.ttl = forward.ttl.saturating_sub(1);
    let Ok(signed_bytes) = serde_json::to_vec(&forward) else {
        return;
    };

    let Some(rc) = state_helpers::routing_context(state) else {
        return;
    };

    let peers: Vec<Vec<u8>> = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else {
            return;
        };
        let Some(ref gossip) = cs.gossip else { return };
        gossip
            .peers
            .iter()
            .filter(|(pk, _)| *pk != &signed.sender_pseudonym)
            .map(|(_, blob)| blob.clone())
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

/// Handle an envelope received via the gossip mesh and process locally.
///
/// Deserializes the inner `CommunityEnvelope` and emits the appropriate
/// `CommunityEvent` to the frontend.
async fn handle_relayed_envelope(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    signed: rekindle_protocol::dht::community::envelope::SignedEnvelope,
) {
    use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

    let envelope: CommunityEnvelope = match serde_json::from_slice(&signed.envelope_bytes) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "invalid relayed envelope");
            return;
        }
    };

    let community_id = signed.community_id.clone();

    match envelope {
        CommunityEnvelope::ChatMessage {
            channel_id,
            message_id,
            author_pseudonym,
            ciphertext,
            mek_generation,
            timestamp,
            reply_to_id,
            ..
        } => {
            // Route through the shared handler which handles decryption, storage, and emit
            let msg = BroadcastNewMessage {
                community_id,
                channel_id,
                message_id,
                sender_pseudonym: author_pseudonym,
                ciphertext,
                mek_generation,
                timestamp,
                reply_to_id,
            };
            handle_broadcast_new_message(app_handle, state, &msg).await;
        }
        CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        } => {
            use crate::channels::CommunityEvent;
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
            ..
        } => {
            use crate::channels::CommunityEvent;
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberPresenceChanged {
                    community_id,
                    pseudonym_key,
                    status,
                    game_name: None,
                    game_id: None,
                    elapsed_seconds: None,
                    server_address: None,
                },
            );
        }
        CommunityEnvelope::Control(payload) => {
            handle_relayed_control(
                app_handle,
                state,
                &community_id,
                &signed.sender_pseudonym,
                payload,
            )
            .await;
        }
    }
}

/// Handle a relayed control payload.
async fn handle_relayed_control(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        // MemberJoinRequest — coordinator receives this via the loopback in
        // handle_app_message. The relay processes it asynchronously (ban check,
        // invite validation, registry write, JoinAccepted delivery) via
        // handle_incoming_envelope → handle_control. After validation succeeds,
        // the relay calls emit_local_member_joined() to persist and emit
        // MemberJoined. We intentionally do NOT emit here to avoid showing
        // members who are subsequently rejected by the relay.
        ControlPayload::MemberJoinRequest { .. } => {}
        ControlPayload::MemberJoined {
            pseudonym_key,
            display_name,
            role_ids,
        } => {
            // Persist to SQLite so get_community_members includes the new member
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let pk = pseudonym_key.clone();
            let dn = display_name.clone();
            let rids = role_ids.clone();
            crate::db_helpers::db_fire(pool.inner(), "persist MemberJoined", move |conn| {
                let role_ids_json =
                    serde_json::to_string(&rids).unwrap_or_else(|_| "[0,1]".into());
                let now = crate::db::timestamp_now();
                conn.execute(
                    "INSERT OR IGNORE INTO community_members \
                     (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                    rusqlite::params![owner_key, cid, pk, dn, role_ids_json, now],
                )?;
                Ok(())
            });

            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberJoined {
                    community_id: community_id.to_string(),
                    pseudonym_key,
                    display_name,
                    role_ids,
                },
            );
        }
        ControlPayload::MemberRemoved { pseudonym_key } => {
            // Remove from SQLite
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let pk = pseudonym_key.clone();
            crate::db_helpers::db_fire(pool.inner(), "persist MemberRemoved", move |conn| {
                conn.execute(
                    "DELETE FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![owner_key, cid, pk],
                )?;
                Ok(())
            });

            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberRemoved {
                    community_id: community_id.to_string(),
                    pseudonym_key,
                },
            );
        }
        ControlPayload::MemberTimedOut {
            pseudonym_key,
            timeout_until,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberTimedOut {
                    community_id: community_id.to_string(),
                    pseudonym_key,
                    timeout_until,
                },
            );
        }
        ControlPayload::MessageEdited {
            channel_id,
            message_id,
            new_ciphertext,
            mek_generation: _,
            edited_at,
        } => {
            // Decrypt the edited message body with the channel/community MEK
            let new_body = {
                let decrypted = {
                    let mek_cache = state.channel_mek_cache.lock();
                    mek_cache
                        .get(&(community_id.to_string(), channel_id.clone()))
                        .map(|mek| mek.decrypt(&new_ciphertext))
                };
                match decrypted {
                    Some(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
                    Some(Err(_)) => "(decryption failed)".to_string(),
                    None => {
                        let mek_cache = state.mek_cache.lock();
                        if let Some(mek) = mek_cache.get(community_id) {
                            match mek.decrypt(&new_ciphertext) {
                                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                                Err(_) => "(decryption failed)".to_string(),
                            }
                        } else {
                            "(no MEK available)".to_string()
                        }
                    }
                }
            };
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MessageEdited {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                    new_body,
                    edited_at,
                },
            );
        }
        ControlPayload::MessageDeleted {
            channel_id,
            message_id,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MessageDeleted {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                },
            );
        }
        ControlPayload::ReactionAdded {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ReactionAdded {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                    emoji,
                    reactor_pseudonym,
                },
            );
        }
        ControlPayload::ReactionRemoved {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ReactionRemoved {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                    emoji,
                    reactor_pseudonym,
                },
            );
        }
        ControlPayload::RolesChanged { roles } => {
            // Convert Vec<serde_json::Value> → Vec<RoleDto>
            let role_dtos: Vec<crate::channels::community_channel::RoleDto> = roles
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::RolesChanged {
                    community_id: community_id.to_string(),
                    roles: role_dtos,
                },
            );
        }
        ControlPayload::MemberRolesChanged {
            pseudonym_key,
            role_ids,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberRolesChanged {
                    community_id: community_id.to_string(),
                    pseudonym_key,
                    role_ids,
                },
            );
        }
        // JoinAccepted — coordinator sent us community data + MEK after our join
        ControlPayload::JoinAccepted {
            mek_encrypted,
            mek_generation,
            channels,
            categories,
            role_ids,
            roles,
            members,
        } => {
            handle_join_accepted(
                app_handle,
                state,
                community_id,
                &mek_encrypted,
                mek_generation,
                &channels,
                &categories,
                &role_ids,
                &roles,
                &members,
            )
            .await;
        }
        // JoinRejected — coordinator denied our join request
        ControlPayload::JoinRejected { reason } => {
            tracing::warn!(
                community = %community_id,
                reason = %reason,
                "join request rejected by coordinator"
            );
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::JoinRejected {
                    community_id: community_id.to_string(),
                    reason,
                },
            );
        }
        // All other control payloads handled by helper function
        other => {
            handle_relayed_control_extended(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                other,
            );
        }
    }
}

/// Extended control payload → frontend event mapping (split for clippy line limit).
fn handle_relayed_control_extended(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::ChannelsUpdated { channels, categories } => {
            let channel_dtos: Vec<crate::channels::community_channel::ChannelInfoFrontendDto> = channels
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
            let category_dtos: Vec<crate::channels::community_channel::CategoryInfoFrontendDto> = categories
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ChannelsUpdated {
                    community_id: community_id.to_string(),
                    channels: channel_dtos,
                    categories: category_dtos,
                },
            );
        }
        ControlPayload::ChannelOverwriteChanged { channel_id } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ChannelOverwriteChanged {
                    community_id: community_id.to_string(),
                    channel_id,
                },
            );
        }
        ControlPayload::MessagePinned {
            channel_id,
            message_id,
            pinned_by,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MessagePinned {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                    pinned_by,
                },
            );
        }
        ControlPayload::MessageUnpinned {
            channel_id,
            message_id,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MessageUnpinned {
                    community_id: community_id.to_string(),
                    channel_id,
                    message_id,
                },
            );
        }
        ControlPayload::InviteCreated {
            code,
            created_by,
            max_uses,
            uses,
            expires_at,
            created_at,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::InviteCreated {
                    community_id: community_id.to_string(),
                    code,
                    created_by,
                    max_uses,
                    uses,
                    expires_at,
                    created_at,
                },
            );
        }
        ControlPayload::InviteRevoked { code } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::InviteRevoked {
                    community_id: community_id.to_string(),
                    code,
                },
            );
        }
        ControlPayload::InviteUsed {
            code,
            new_use_count,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::InviteUsed {
                    community_id: community_id.to_string(),
                    code,
                    new_use_count,
                },
            );
        }
        ControlPayload::EventCreated { event } => {
            if let Ok(dto) = serde_json::from_value::<crate::channels::community_channel::EventInfoDto>(event) {
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::EventCreated {
                        community_id: community_id.to_string(),
                        event: dto,
                    },
                );
            }
        }
        ControlPayload::EventUpdated { event } => {
            if let Ok(dto) = serde_json::from_value::<crate::channels::community_channel::EventInfoDto>(event) {
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::EventUpdated {
                        community_id: community_id.to_string(),
                        event: dto,
                    },
                );
            }
        }
        ControlPayload::EventDeleted { event_id } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::EventDeleted {
                    community_id: community_id.to_string(),
                    event_id,
                },
            );
        }
        ControlPayload::EventRsvpChanged {
            event_id,
            pseudonym_key,
            status,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::EventRsvpChanged {
                    community_id: community_id.to_string(),
                    event_id,
                    pseudonym_key,
                    status,
                },
            );
        }
        ControlPayload::ThreadCreated { thread } => {
            if let Ok(dto) = serde_json::from_value::<crate::channels::community_channel::ThreadInfoDto>(thread) {
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::ThreadCreated {
                        community_id: community_id.to_string(),
                        thread: dto,
                    },
                );
            }
        }
        ControlPayload::ThreadArchived { thread_id, archived } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ThreadArchived {
                    community_id: community_id.to_string(),
                    thread_id,
                    archived,
                },
            );
        }
        ControlPayload::GameServerAdded { server } => {
            if let Ok(dto) = serde_json::from_value::<crate::channels::community_channel::GameServerInfoDto>(server) {
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::GameServerAdded {
                        community_id: community_id.to_string(),
                        server: dto,
                    },
                );
            }
        }
        ControlPayload::GameServerRemoved { server_id } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::GameServerRemoved {
                    community_id: community_id.to_string(),
                    server_id,
                },
            );
        }
        ControlPayload::MEKRotated { new_generation } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MekRotated {
                    community_id: community_id.to_string(),
                    new_generation,
                },
            );
        }
        ControlPayload::KickedNotification => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::Kicked {
                    community_id: community_id.to_string(),
                },
            );
        }
        ControlPayload::ThreadMessageReceived {
            thread_id,
            message_id,
            sender_pseudonym,
            ciphertext,
            mek_generation,
            timestamp,
            reply_to_id,
        } => {
            let body = {
                let mek_cache = state.mek_cache.lock();
                match decrypt_with_cached_mek(&mek_cache, community_id, &ciphertext, mek_generation)
                {
                    MekDecryptResult::Decrypted(text) => text,
                    _ => String::new(),
                }
            };
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ThreadMessageReceived {
                    community_id: community_id.to_string(),
                    thread_id,
                    message_id,
                    sender_pseudonym,
                    body,
                    timestamp,
                    reply_to_id,
                },
            );
        }
        ControlPayload::EventReminder {
            event_id,
            title,
            minutes_until_start,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::EventReminder {
                    community_id: community_id.to_string(),
                    event_id,
                    title,
                    minutes_until_start,
                },
            );
        }
        // Coordinator lifecycle / ack payloads — no frontend event needed
        ControlPayload::CoordinatorHeartbeat { .. }
        | ControlPayload::ElectionClaim { .. }
        | ControlPayload::Ok
        | ControlPayload::Error { .. }
        | ControlPayload::MessageSent { .. }
        | ControlPayload::AutoModBlocked { .. }
        | ControlPayload::OnboardingQuestions { .. } => {}
        ControlPayload::SystemMessage { body, timestamp } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::SystemMessage {
                    community_id: community_id.to_string(),
                    body,
                    timestamp,
                },
            );
        }
        ControlPayload::RaidAlert { active } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::RaidAlert {
                    community_id: community_id.to_string(),
                    active,
                },
            );
        }
        ControlPayload::ChannelLockdown { locked } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ChannelLockdown {
                    community_id: community_id.to_string(),
                    locked,
                },
            );
        }
        // Phase 2 gossip mesh control payloads
        other => {
            handle_gossip_control_payloads(app_handle, state, community_id, sender_pseudonym, other);
        }
    }
}

/// Handle Phase 2 gossip mesh control payloads (admin delegation, sync, coordinator announce).
fn handle_gossip_control_payloads(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::AdminKeypairGrant {
            wrapped_manifest_keypair,
            wrapped_slot_seed,
        } => {
            handle_admin_keypair_grant(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                &wrapped_manifest_keypair,
                &wrapped_slot_seed,
            );
        }
        ControlPayload::SlotKeypairGrant {
            slot_index,
            segment_index,
            wrapped_slot_keypair,
        } => {
            handle_slot_keypair_grant(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                slot_index,
                segment_index,
                &wrapped_slot_keypair,
            );
        }
        ControlPayload::SyncRequest {
            channel_id,
            since_timestamp,
        } => {
            handle_sync_request(app_handle, state, community_id, &channel_id, since_timestamp);
        }
        ControlPayload::SyncResponse {
            channel_id,
            messages,
        } => {
            handle_sync_response(app_handle, state, community_id, &channel_id, &messages);
        }
        ControlPayload::CoordinatorAnnounce {
            pseudonym_key,
            route_blob,
            epoch,
        } => {
            let mut communities = state.communities.write();
            if let Some(c) = communities.get_mut(community_id) {
                c.coordinator_pseudonym = Some(pseudonym_key.clone());
                c.coordinator_route_blob = Some(route_blob);
                c.coordinator_epoch = epoch;
            }
            tracing::info!(
                community = %community_id,
                coordinator = %pseudonym_key,
                epoch,
                "coordinator announced"
            );
        }
        _ => {
            tracing::debug!(
                community = %community_id,
                "received relayed control payload (not yet mapped to event)"
            );
        }
    }
}

/// Unwrap and persist admin keypair grant (manifest keypair + slot seed).
fn handle_admin_keypair_grant(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    wrapped_manifest_keypair: &[u8],
    wrapped_slot_seed: &[u8],
) {
    use rekindle_crypto::group::mek_distribution::unwrap_mek;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

    // Derive our community pseudonym signing key for ECDH
    let Some(secret) = state.identity_secret.lock().as_ref().copied() else {
        tracing::warn!("no identity secret — cannot unwrap admin keypair grant");
        return;
    };
    let my_signing_key = derive_community_pseudonym(&secret, community_id);

    // Parse sender's pseudonym public key bytes
    let Ok(sender_bytes) = hex::decode(sender_pseudonym) else {
        tracing::warn!("invalid sender pseudonym hex in admin keypair grant");
        return;
    };
    let Ok(sender_pub): Result<[u8; 32], _> = sender_bytes.try_into() else {
        tracing::warn!("sender pseudonym wrong length in admin keypair grant");
        return;
    };

    // Unwrap manifest keypair
    let manifest_kp_bytes = match unwrap_mek(&my_signing_key, &sender_pub, wrapped_manifest_keypair)
    {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap manifest keypair");
            return;
        }
    };
    let manifest_kp_str = String::from_utf8_lossy(&manifest_kp_bytes).to_string();

    // Unwrap slot seed
    let slot_seed_bytes = match unwrap_mek(&my_signing_key, &sender_pub, wrapped_slot_seed) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap slot seed");
            return;
        }
    };

    // Persist to CommunityState
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.manifest_owner_keypair = Some(manifest_kp_str.clone());
            c.slot_seed = Some(hex::encode(&slot_seed_bytes));
        }
    }

    // Persist to Stronghold
    let seed_hex = hex::encode(&slot_seed_bytes);
    let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let ks = ks_handle.lock();
    if let Some(ref keystore) = *ks {
        crate::keystore::persist_manifest_keypair(keystore, community_id, &manifest_kp_str);
        crate::keystore::persist_slot_seed(keystore, community_id, &seed_hex);
    }

    tracing::info!(community = %community_id, "admin keypair grant accepted and persisted");
}

/// Unwrap and persist slot keypair grant.
fn handle_slot_keypair_grant(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    slot_index: u32,
    segment_index: u32,
    wrapped_slot_keypair: &[u8],
) {
    use rekindle_crypto::group::mek_distribution::unwrap_mek;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

    let Some(secret) = state.identity_secret.lock().as_ref().copied() else {
        tracing::warn!("no identity secret — cannot unwrap slot keypair grant");
        return;
    };
    let my_signing_key = derive_community_pseudonym(&secret, community_id);

    let Ok(sender_bytes) = hex::decode(sender_pseudonym) else {
        tracing::warn!("invalid sender pseudonym hex in slot keypair grant");
        return;
    };
    let Ok(sender_pub): Result<[u8; 32], _> = sender_bytes.try_into() else {
        tracing::warn!("sender pseudonym wrong length in slot keypair grant");
        return;
    };

    let slot_kp_bytes = match unwrap_mek(&my_signing_key, &sender_pub, wrapped_slot_keypair) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap slot keypair");
            return;
        }
    };
    let slot_kp_str = String::from_utf8_lossy(&slot_kp_bytes).to_string();

    // Persist to CommunityState
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.slot_keypair = Some(slot_kp_str.clone());
        }
    }

    // Persist to Stronghold
    let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let ks = ks_handle.lock();
    if let Some(ref keystore) = *ks {
        crate::keystore::persist_slot_keypair(keystore, community_id, &slot_kp_str);
    }

    tracing::info!(
        community = %community_id,
        slot_index, segment_index,
        "slot keypair grant accepted and persisted"
    );
}

/// Handle a sync request — respond with messages from local SQLite.
fn handle_sync_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    since_timestamp: u64,
) {
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
    let cid = community_id.to_string();
    let ch = channel_id.to_string();
    let ch_for_envelope = channel_id.to_string();
    let since_ts = since_timestamp.cast_signed();
    let state = Arc::clone(state);
    let pool = pool.inner().clone();

    tokio::spawn(async move {
        let messages: Vec<serde_json::Value> =
            crate::db_helpers::db_call(&pool, move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT sender_key, body, timestamp, mek_generation, lamport_ts \
                     FROM messages \
                     WHERE owner_key = ? AND conversation_id = ? \
                       AND conversation_type = 'channel' AND timestamp >= ? \
                     ORDER BY timestamp ASC LIMIT 500",
                )?;
                let rows = stmt.query_map(rusqlite::params![owner_key, ch, since_ts], |row| {
                    Ok(serde_json::json!({
                        "sender_key": row.get::<_, String>(0)?,
                        "body": row.get::<_, String>(1)?,
                        "timestamp": row.get::<_, i64>(2)?,
                        "mek_generation": row.get::<_, Option<i64>>(3)?,
                        "lamport_ts": row.get::<_, Option<i64>>(4)?,
                    }))
                })?;
                Ok(rows.filter_map(std::result::Result::ok).collect::<Vec<_>>())
            })
            .await
            .unwrap_or_default();

        if messages.is_empty() {
            return;
        }

        tracing::debug!(
            community = %cid,
            count = messages.len(),
            "responding to sync request"
        );

        let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::SyncResponse {
                channel_id: ch_for_envelope,
                messages,
            },
        );
        let _ = crate::commands::community::send_to_mesh(&state, &cid, &envelope);
    });
}

/// Handle a sync response — merge messages into local SQLite and emit to frontend.
fn handle_sync_response(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    messages: &[serde_json::Value],
) {
    if messages.is_empty() {
        return;
    }

    tracing::info!(
        community = %community_id,
        channel = %channel_id,
        count = messages.len(),
        "merging sync response messages"
    );

    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();

    for msg in messages {
        let sender = msg["sender_key"].as_str().unwrap_or_default().to_string();
        let body = msg["body"].as_str().unwrap_or_default().to_string();
        let ts = msg["timestamp"].as_i64().unwrap_or_default();
        let mek_gen = msg["mek_generation"].as_i64();
        let ok = owner_key.clone();
        let ch = channel_id.to_string();
        db_fire(pool.inner(), "store sync message", move |conn| {
            // Use INSERT OR IGNORE to skip duplicates (dedup index handles this)
            conn.execute(
                "INSERT OR IGNORE INTO messages \
                 (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read, mek_generation) \
                 VALUES (?, ?, 'channel', ?, ?, ?, 0, ?)",
                rusqlite::params![ok, ch, sender, body, ts, mek_gen],
            )?;
            Ok(())
        });
    }

    // Emit a catch-up event so the frontend can refresh the channel
    let _ = app_handle.emit(
        "community-event",
        crate::channels::CommunityEvent::SyncComplete {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message_count: messages.len(),
        },
    );
}

/// Handle a JoinAccepted response from the coordinator.
///
/// Updates local community state with fresh MEK, roles, channels, and members.
/// Persists member list to SQLite so `get_community_members` returns all members.
async fn handle_join_accepted(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    mek_wire_bytes: &[u8],
    mek_generation: u64,
    channels: &[serde_json::Value],
    _categories: &[serde_json::Value],
    role_ids: &[u32],
    roles: &[serde_json::Value],
    members: &[serde_json::Value],
) {
    use crate::channels::CommunityEvent;
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_protocol::dht::community::types::MemberSummary;

    // 1. Restore and cache the MEK
    if !mek_wire_bytes.is_empty() {
        if let Some(mek) = MediaEncryptionKey::from_wire_bytes(mek_wire_bytes) {
            let gen = mek.generation();
            state
                .mek_cache
                .lock()
                .insert(community_id.to_string(), mek);
            tracing::info!(
                community = %community_id,
                mek_generation = gen,
                "cached MEK from JoinAccepted"
            );
        } else {
            tracing::warn!(
                community = %community_id,
                "JoinAccepted contained invalid MEK wire bytes"
            );
        }
    }

    // 2. Parse members from coordinator
    let parsed_members: Vec<MemberSummary> = members
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();

    // 3. Parse roles from coordinator
    let parsed_roles: Vec<crate::state::RoleDefinition> = roles
        .iter()
        .filter_map(|v| {
            let entry: rekindle_protocol::dht::community::types::RoleEntryV2 =
                serde_json::from_value(v.clone()).ok()?;
            Some(crate::state::RoleDefinition {
                id: entry.id,
                name: entry.name,
                color: entry.color,
                permissions: entry.permissions,
                position: entry.position,
                hoist: entry.hoist,
                mentionable: entry.mentionable,
            })
        })
        .collect();

    // 4. Update community state with correct mek_generation, role_ids, roles, and channels
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.mek_generation = mek_generation;
            if !role_ids.is_empty() {
                cs.my_role_ids = role_ids.to_vec();
            }
            if !parsed_roles.is_empty() {
                cs.roles = parsed_roles;
            }

            // Update channel list if provided
            if !channels.is_empty() {
                let mut updated_channels = Vec::new();
                for ch_val in channels {
                    if let Ok(ch) =
                        serde_json::from_value::<crate::state::ChannelInfo>(ch_val.clone())
                    {
                        updated_channels.push(ch);
                    }
                }
                if !updated_channels.is_empty() {
                    cs.channels = updated_channels;
                }
            }
        }
    }

    // 5. Persist members to SQLite so get_community_members returns all members.
    //    Use db_call (blocking) so the write completes BEFORE we emit the frontend
    //    event — otherwise getCommunityMembers may return an empty list.
    if !parsed_members.is_empty() {
        let pool: tauri::State<'_, DbPool> = app_handle.state();
        let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
        let cid = community_id.to_string();
        let members_for_db = parsed_members.clone();
        let result = crate::db_helpers::db_call(pool.inner(), move |conn| {
            for m in &members_for_db {
                let role_ids_json =
                    serde_json::to_string(&m.role_ids).unwrap_or_else(|_| "[0,1]".into());
                conn.execute(
                    "INSERT OR REPLACE INTO community_members \
                     (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at, subkey_index, onboarding_complete, timeout_until) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        owner_key,
                        cid,
                        m.pseudonym_key,
                        m.display_name,
                        role_ids_json,
                        m.joined_at.cast_signed(),
                        m.subkey_index,
                        i32::from(m.onboarding_complete),
                        m.timeout_until.map(u64::cast_signed),
                    ],
                )?;
            }
            Ok(())
        })
        .await;
        if let Err(e) = result {
            tracing::warn!(community = %community_id, error = %e, "failed to persist JoinAccepted members to SQLite");
        }
    }

    // 6. Persist MEK to Stronghold so it survives app restarts.
    //    The persist_mek call in join_community_command runs BEFORE JoinAccepted
    //    arrives (join is async fire-and-forget), so it finds nothing in mek_cache.
    //    We must persist here, after the MEK is cached.
    if !mek_wire_bytes.is_empty() {
        let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
        let ks = ks_handle.lock();
        if let Some(ref keystore) = *ks {
            let mek_cache = state.mek_cache.lock();
            if let Some(mek) = mek_cache.get(community_id) {
                crate::keystore::persist_mek(keystore, community_id, mek);
                tracing::debug!(community = %community_id, "persisted MEK to Stronghold after JoinAccepted");
            }
        }
    }

    // 7. Notify frontend
    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::JoinAccepted {
            community_id: community_id.to_string(),
        },
    );

    tracing::info!(
        community = %community_id,
        mek_generation,
        role_ids = ?role_ids,
        member_count = parsed_members.len(),
        "JoinAccepted processed — community state updated"
    );
}

/// Decrypt result for community message MEK decryption attempts.
enum MekDecryptResult {
    Decrypted(String),
    NeedRefresh,
    Failed,
}

/// Parameters for handling a new community message broadcast.
struct BroadcastNewMessage {
    community_id: String,
    channel_id: String,
    message_id: String,
    sender_pseudonym: String,
    ciphertext: Vec<u8>,
    mek_generation: u64,
    timestamp: u64,
    reply_to_id: Option<String>,
}

/// Handle a `NewMessage` community broadcast: decrypt, store, and emit.
async fn handle_broadcast_new_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    msg: &BroadcastNewMessage,
) {
    // Skip messages we sent ourselves (already echoed locally in send_channel_message)
    {
        let communities = state.communities.read();
        if let Some(community) = communities.get(&msg.community_id) {
            if community.my_pseudonym_key.as_deref() == Some(&msg.sender_pseudonym) {
                return;
            }
        }
    }

    let first_attempt = {
        let mek_cache = state.mek_cache.lock();
        decrypt_with_cached_mek(
            &mek_cache,
            &msg.community_id,
            &msg.ciphertext,
            msg.mek_generation,
        )
    }; // guard dropped here — safe to .await

    let body = match first_attempt {
        MekDecryptResult::Decrypted(body) => body,
        MekDecryptResult::Failed => return,
        MekDecryptResult::NeedRefresh => {
            fetch_mek_from_server(app_handle, state, &msg.community_id).await;

            // Retry with refreshed MEK
            let mek_cache = state.mek_cache.lock();
            if let MekDecryptResult::Decrypted(body) = decrypt_with_cached_mek(
                &mek_cache,
                &msg.community_id,
                &msg.ciphertext,
                msg.mek_generation,
            ) {
                body
            } else {
                tracing::warn!("MEK still mismatched after refresh — dropping message");
                return;
            }
        }
    };

    // Store locally
    let owner_key = state_helpers::owner_key_or_default(state);

    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let cid = msg.channel_id.clone();
    let spn = msg.sender_pseudonym.clone();
    let body_text = body.clone();
    let ts = msg.timestamp.cast_signed();
    let mg = msg.mek_generation.cast_signed();
    db_fire(pool.inner(), "store community message", move |conn| {
        crate::message_repo::insert_channel_message(conn, &owner_key, &cid, &spn, &body_text, ts, false, Some(mg))
    });

    // Emit to frontend
    let event = crate::channels::ChatEvent::MessageReceived {
        from: msg.sender_pseudonym.clone(),
        body,
        timestamp: msg.timestamp,
        conversation_id: msg.channel_id.clone(),
        server_message_id: Some(msg.message_id.clone()),
        reply_to_id: msg.reply_to_id.clone(),
    };
    let _ = app_handle.emit("chat-event", &event);
}

/// Try to decrypt ciphertext using the cached MEK for a community.
fn decrypt_with_cached_mek(
    mek_cache: &std::collections::HashMap<
        String,
        rekindle_crypto::group::media_key::MediaEncryptionKey,
    >,
    community_id: &str,
    ciphertext: &[u8],
    mek_generation: u64,
) -> MekDecryptResult {
    match mek_cache.get(community_id) {
        Some(mek) if mek.generation() == mek_generation => match mek.decrypt(ciphertext) {
            Ok(plaintext) => {
                MekDecryptResult::Decrypted(String::from_utf8(plaintext).unwrap_or_default())
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to decrypt community message");
                MekDecryptResult::Failed
            }
        },
        Some(mek) => {
            tracing::warn!(
                have = mek.generation(),
                need = mek_generation,
                "MEK generation mismatch — fetching updated MEK from server"
            );
            MekDecryptResult::NeedRefresh
        }
        None => {
            tracing::warn!(community = %community_id, "no MEK cached for community");
            MekDecryptResult::Failed
        }
    }
}


/// Fetch the current MEK from the community server via `RequestMEK` RPC.
///
/// Updates `mek_cache` and community state on success. Also persists the
/// updated MEK to Stronghold so it survives restarts.
pub(super) async fn fetch_mek_from_server(
    _app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
) {
    use rekindle_protocol::dht::community::envelope::{ControlPayload, CommunityEnvelope};

    // Send a RequestMEK to the coordinator via the v2 envelope path.
    // The coordinator will respond asynchronously by relaying a JoinAccepted
    // (or a dedicated MEK delivery) back through the relay, which will be
    // processed by handle_relayed_control in the dispatch loop.
    let result = crate::commands::community::send_to_coordinator(
        state,
        community_id,
        CommunityEnvelope::Control(ControlPayload::RequestMEK),
    )
    .await;

    if let Err(e) = result {
        tracing::warn!(error = %e, community = %community_id, "failed to send RequestMEK to coordinator");
    }
}

/// Handle an incoming `AppCall` — process the message, then reply with ACK.
async fn handle_app_call(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    call: veilid_core::VeilidAppCall,
) {
    let call_id = call.id();
    tracing::debug!(call_id = %call_id, "app_call received");

    // Route the call through message handling (same as app_message)
    // then reply with an acknowledgment
    let message = call.message().to_vec();
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    super::message_service::handle_incoming_message(app_handle, state, pool.inner(), &message)
        .await;

    // Reply with ACK so the caller's app_call future resolves.
    if let Some(api) = state_helpers::veilid_api(state) {
        if let Err(e) = api.app_call_reply(call_id, b"ACK".to_vec()).await {
            tracing::warn!(error = %e, "failed to reply to app_call");
        }
    }
}

/// Attempt to re-establish a DHT watch for the friend owning `dht_key`.
/// Falls back to adding to `unwatched_friends` poll set if re-watch fails.
async fn try_rewatch_friend(state: &Arc<AppState>, dht_key: &str) {
    let friend_info =
        { state_helpers::friend_for_dht_key(state, dht_key).map(|fk| (fk, dht_key.to_string())) };
    let Some((fk, dk)) = friend_info else { return };
    if super::presence_service::watch_friend(state, &fk, &dk)
        .await
        .is_err()
    {
        state.unwatched_friends.write().insert(fk);
    }
}

/// Attempt to re-establish a DHT watch for a community record.
///
/// Called when a `VeilidValueChange` with empty subkeys arrives for a key
/// that belongs to a community (not a friend). Re-watches subkeys 0-3, 5-6
/// (metadata, channels, roster, roles, MEK, server route).
async fn try_rewatch_community(state: &Arc<AppState>, dht_key: &str) {
    let community_id = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|c| c.dht_record_key.as_deref() == Some(dht_key))
            .map(|c| c.id.clone())
    };
    let Some(community_id) = community_id else {
        return;
    };
    let Some(rc) = state_helpers::routing_context(state) else {
        return;
    };
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    if let Err(e) = mgr.watch_record(dht_key, &[0, 1, 2, 3, 5, 6]).await {
        tracing::warn!(
            community = %community_id, error = %e,
            "failed to re-establish community DHT watch"
        );
    } else {
        tracing::info!(community = %community_id, "re-established community DHT watch");
    }
}

/// Handle a DHT `ValueChange` notification by forwarding to the presence service.
///
/// When the inline value is `None` (Veilid doesn't always include it), we fetch
/// each changed subkey's value from DHT individually.  The previous code silently
/// passed an empty vec which caused `parse_status` to return `None`, dropping
/// the status change entirely — this was why automated status updates (auto-away,
/// offline on logout) weren't visible to friends.
async fn handle_value_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    change: veilid_core::VeilidValueChange,
) {
    let key = change.key.to_string();

    // Detect watch death: empty subkeys means the watch has died.
    // Per veilid-core VeilidValueChange docs: "If the subkey range is empty,
    // any watch present on the value has died."
    if change.subkeys.is_empty() {
        tracing::warn!(key = %key, count = change.count, "DHT watch died — attempting immediate re-watch");
        try_rewatch_friend(state, &key).await;
        try_rewatch_community(state, &key).await;
        return;
    }

    // count == 0 with non-empty subkeys means this is the last change notification
    // before the watch expires. Process the change AND schedule a re-watch.
    if change.count == 0 {
        tracing::info!(key = %key, "DHT watch expiring (count=0) — attempting immediate re-watch");
        try_rewatch_friend(state, &key).await;
        try_rewatch_community(state, &key).await;
        // Fall through to process the change below
    }

    let subkeys: Vec<u32> = change.subkeys.iter().collect();
    // Per veilid-core docs: "The (optional) value data for the first subkey
    // in the subkeys range. If 'subkeys' is not a single value, other values
    // than the first value must be retrieved with RoutingContext::get_dht_value()."
    let first_subkey = subkeys.first().copied();
    let inline_value = change.value.as_ref().map(|v| v.data().to_vec());
    tracing::debug!(
        key = %key,
        subkeys = ?subkeys,
        has_inline = inline_value.is_some(),
        "DHT value changed"
    );

    // Get routing context for fetching subkey values when not provided inline
    // NOTE: We intentionally don't filter on is_attached here — the routing context
    // is still usable for local DHT reads even during brief detach windows.
    let routing_context = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.routing_context.clone())
    };

    // Forward coordinator-relevant changes to the coordinator service.
    // Check if this DHT key belongs to a community's manifest record.
    {
        let coordinator_services = state.coordinator_services.read();
        let communities = state.communities.read();
        for (cid, cs) in communities.iter() {
            let manifest_matches = cs.manifest_key.as_deref() == Some(&key)
                || cs.id == key;
            if manifest_matches {
                if let Some(handle) = coordinator_services.get(cid) {
                    for &subkey in &subkeys {
                        let value = if Some(subkey) == first_subkey {
                            inline_value.clone()
                        } else {
                            None
                        };
                        let _ = handle.value_change_tx.try_send(
                            super::coordinator::CoordinatorValueChange { subkey, value },
                        );
                    }
                }
                break;
            }
        }
    }

    for &subkey in &subkeys {
        // Inline value is only valid for the first subkey in the range;
        // all other subkeys must be fetched from DHT individually.
        let use_inline = Some(subkey) == first_subkey;
        let value = if use_inline && inline_value.is_some() {
            inline_value.clone().unwrap()
        } else if let Some(ref rc) = routing_context {
            match rc.get_dht_value(change.key.clone(), subkey, true).await {
                Ok(Some(v)) => v.data().to_vec(),
                Ok(None) => {
                    tracing::debug!(subkey, key = %key, "subkey has no value");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(subkey, key = %key, error = %e, "failed to fetch subkey");
                    continue;
                }
            }
        } else {
            tracing::debug!(subkey, "no routing context to fetch subkey value");
            continue;
        };
        super::presence_service::handle_value_change(app_handle, state, &key, &[subkey], &value)
            .await;
    }
}

/// Move all friends with DHT keys into the unwatched set, forcing re-watch.
/// Called on network reconnection when all existing Veilid DHT watches are dead.
fn invalidate_all_watches(state: &Arc<AppState>) {
    let friend_keys: Vec<String> = {
        let friends = state.friends.read();
        friends
            .values()
            .filter(|f| f.dht_record_key.is_some())
            .map(|f| f.public_key.clone())
            .collect()
    };
    if !friend_keys.is_empty() {
        let mut unwatched = state.unwatched_friends.write();
        for key in &friend_keys {
            unwatched.insert(key.clone());
        }
        tracing::info!(
            count = friend_keys.len(),
            "invalidated all friend watches for re-establishment"
        );
    }
}

/// Handle a network attachment state change — update node state and notify the frontend.
///
/// Detects detached→attached transitions (reconnection) and triggers an immediate
/// friend resync to re-establish DHT watches that died during the disconnection.
fn handle_attachment(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    attachment: &veilid_core::VeilidStateAttachment,
) {
    let attached = attachment.state.is_attached();
    let public_internet_ready = attachment.public_internet_ready;
    let state_str = attachment.state.to_string();
    tracing::info!(
        state = %state_str,
        public_internet_ready,
        "network attachment changed"
    );

    // Detect transition: was detached, now attached
    let was_attached = state_helpers::is_attached(state);
    let reconnected = !was_attached && attached;

    {
        if let Some(ref mut node) = *state.node.write() {
            node.attachment_state = state_str;
            node.is_attached = attached;
            node.public_internet_ready = public_internet_ready;
        }
    }
    // Propagate readiness via watch channel — never loses signals, no TOCTOU race
    let _ = state.network_ready_tx.send(public_internet_ready);

    // Push structured event so the frontend's NetworkIndicator can react immediately
    emit_network_status(app_handle, state);

    let status = if attached {
        "connected"
    } else {
        "disconnected"
    };
    let notification = NotificationEvent::SystemAlert {
        title: "Network".to_string(),
        body: format!("Veilid network {status}"),
    };
    let _ = app_handle.emit("notification-event", &notification);

    // On reconnection: invalidate all watches and resync friend statuses in background.
    // Gate on public_internet_ready (DHT must be usable) and identity (must be logged in).
    // This filters out the initial startup false→true transition (no identity yet) and
    // intermediate attachment states where DHT isn't available.
    // The resync runs via tokio::spawn so the dispatch loop is never blocked.
    if reconnected && public_internet_ready && state.identity.read().is_some() {
        tracing::info!("network reconnected — invalidating watches and triggering friend resync");
        invalidate_all_watches(state);
        let state = state.clone();
        let app_handle = app_handle.clone();
        tokio::spawn(async move {
            let _ = super::sync_service::sync_friends_now(&state, &app_handle).await;
        });
    }
}

/// Handle a route change — re-allocate our private route if it died, and
/// invalidate cached peer routes.
async fn handle_route_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    change: &veilid_core::VeilidRouteChange,
) {
    tracing::debug!(
        dead_routes = change.dead_routes.len(),
        dead_remote_routes = change.dead_remote_routes.len(),
        "route change event"
    );

    // Check if our specific private route died (not just any route)
    let our_route_died = {
        let rm = state.routing_manager.read();
        rm.as_ref().is_some_and(|handle| {
            handle
                .manager
                .route_id()
                .is_some_and(|our_id| change.dead_routes.contains(&our_id))
        })
    };

    if our_route_died {
        // The route is already dead — forget it (don't call release, which would
        // hit Veilid's "Invalid argument" error for an already-expired route).
        // Then allocate a fresh one.
        {
            let mut rm = state.routing_manager.write();
            if let Some(ref mut handle) = *rm {
                handle.manager.forget_private_route();
            }
        }
        allocate_fresh_private_route(app_handle, state).await;
    }

    // Invalidate cached peer routes that died (selective — only affected peers)
    if !change.dead_remote_routes.is_empty() {
        {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager
                    .invalidate_dead_routes(&change.dead_remote_routes);
            }
        }

        // Clear coordinator route blobs that reference dead routes.
        let dead_set: std::collections::HashSet<_> =
            change.dead_remote_routes.iter().collect();
        let api = {
            let node = state.node.read();
            node.as_ref().map(|n| n.api.clone())
        };
        if let Some(api) = api {
            let mut communities = state.communities.write();
            for community in communities.values_mut() {
                if let Some(ref blob) = community.coordinator_route_blob {
                    if let Ok(route_id) = api.import_remote_private_route(blob.clone()) {
                        if dead_set.contains(&route_id) {
                            tracing::info!(
                                community = %community.id,
                                "clearing dead coordinator route blob"
                            );
                            community.coordinator_route_blob = None;
                        }
                    }
                }
            }
        }
    }
}

/// Allocate a new private route, then release the old one (make-before-break).
///
/// Used by the periodic refresh loop where the old route is still valid.
/// By allocating the new route FIRST, we avoid a window where `NodeHandle.route_blob`
/// holds a stale (released) blob. If allocation fails, the old route stays active.
pub(crate) async fn reallocate_private_route(app_handle: &tauri::AppHandle, state: &Arc<AppState>) {
    let Some(api) = state_helpers::veilid_api(state) else {
        return;
    };

    // Allocate new route FIRST (make-before-break)
    let new_route = match api.new_private_route().await {
        Ok(rb) => rb,
        Err(e) => {
            tracing::warn!(error = %e, "route refresh: failed to allocate new route — keeping old");
            return;
        }
    };

    // New route allocated — NOW release the old one and store the new one
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release old private route");
            }
            handle
                .manager
                .set_allocated_route(new_route.route_id.clone(), new_route.blob.clone());
        }
    }
    // Update NodeHandle
    if let Some(ref mut nh) = *state.node.write() {
        nh.route_blob = Some(new_route.blob.clone());
    }

    // Notify frontend
    emit_network_status(app_handle, state);

    // Re-publish route blob to DHT profile subkey 6
    if let Err(e) =
        super::message_service::push_profile_update(state, 6, new_route.blob.clone()).await
    {
        tracing::warn!(error = %e, "failed to re-publish route blob to DHT");
    }

    // Also update mailbox subkey 0 with the fresh route blob
    let mailbox_key = {
        let node = state.node.read();
        node.as_ref().and_then(|nh| nh.mailbox_dht_key.clone())
    };
    if let Some(mailbox_key) = mailbox_key {
        // NOTE: We intentionally don't filter on is_attached here — routing context
        // is Arc-based and always valid once created.
        let rc = {
            let node = state.node.read();
            node.as_ref().map(|nh| nh.routing_context.clone())
        };
        if let Some(rc) = rc {
            if let Err(e) = rekindle_protocol::dht::mailbox::update_mailbox_route(
                &rc,
                &mailbox_key,
                &new_route.blob,
            )
            .await
            {
                tracing::warn!(error = %e, "failed to update mailbox route blob");
            }
        }
    }

    tracing::info!("re-allocated private route (make-before-break)");
}

/// Allocate a fresh private route and publish it to DHT.
///
/// Assumes the old route has already been released or forgotten. Called by
/// both `reallocate_private_route` (periodic refresh) and `handle_route_change`
/// (dead route recovery).
async fn allocate_fresh_private_route(app_handle: &tauri::AppHandle, state: &Arc<AppState>) {
    let Some(api) = state_helpers::veilid_api(state) else {
        return;
    };

    match api.new_private_route().await {
        Ok(route_blob) => {
            // Store route info back in the routing manager
            {
                let mut rm = state.routing_manager.write();
                if let Some(ref mut handle) = *rm {
                    handle
                        .manager
                        .set_allocated_route(route_blob.route_id.clone(), route_blob.blob.clone());
                }
            }
            // Also store on node handle
            if let Some(ref mut nh) = *state.node.write() {
                nh.route_blob = Some(route_blob.blob.clone());
            }
            // Notify the frontend immediately about the new route
            emit_network_status(app_handle, state);

            // Re-publish route blob to DHT profile subkey 6
            if let Err(e) =
                super::message_service::push_profile_update(state, 6, route_blob.blob.clone()).await
            {
                tracing::warn!(error = %e, "failed to re-publish route blob to DHT");
            }

            // Also update mailbox subkey 0 with the fresh route blob
            let mailbox_key = {
                let node = state.node.read();
                node.as_ref().and_then(|nh| nh.mailbox_dht_key.clone())
            };
            if let Some(mailbox_key) = mailbox_key {
                let rc = {
                    let node = state.node.read();
                    node.as_ref().map(|nh| nh.routing_context.clone())
                };
                if let Some(rc) = rc {
                    if let Err(e) = rekindle_protocol::dht::mailbox::update_mailbox_route(
                        &rc,
                        &mailbox_key,
                        &route_blob.blob,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "failed to update mailbox route blob");
                    }
                }
            }

            tracing::info!("re-allocated private route");
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to allocate private route");
        }
    }
}

/// Periodically re-allocate our private route to prevent silent expiration.
///
/// Veilid routes expire after ~5 minutes and `RouteChange` events can be missed.
/// This loop proactively re-allocates every 120 seconds to ensure peers can
/// always reach us. Spawned as a background task during login and aborted on logout.
pub(crate) async fn route_refresh_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
    // Skip the immediate first tick (route was just allocated at login)
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Only refresh if we have a node and it's attached
                let should_refresh = {
                    let node = state.node.read();
                    node.as_ref().is_some_and(|nh| nh.is_attached && nh.route_blob.is_some())
                };
                if should_refresh {
                    tracing::debug!("proactive route refresh: re-allocating private route");
                    reallocate_private_route(&app_handle, &state).await;

                    // For communities where we ARE the coordinator, immediately write
                    // the new route blob to the manifest so joiners don't get stale routes.
                    // For other communities, re-announce presence to the coordinator.
                    let all_community_ids: Vec<String> = {
                        let communities = state.communities.read();
                        communities.keys().cloned().collect()
                    };
                    for community_id in &all_community_ids {
                        let is_coordinator = {
                            let services = state.coordinator_services.read();
                            services
                                .get(community_id)
                                .is_some_and(super::coordinator::CoordinatorServiceHandle::is_coordinator)
                        };
                        if is_coordinator {
                            // Write heartbeat immediately with new route blob
                            if let Err(e) = super::coordinator::heartbeat::write_heartbeat_now(
                                &state,
                                community_id,
                            )
                            .await
                            {
                                tracing::warn!(
                                    community = %community_id,
                                    error = %e,
                                    "failed to write immediate heartbeat after route refresh"
                                );
                            }
                        } else {
                            // Re-announce presence to the coordinator
                            let _ = crate::services::community_service::rejoin_community(
                                &state,
                                community_id,
                            )
                            .await;
                        }
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::debug!("route refresh loop shutting down");
                break;
            }
        }
    }
}

/// Initialize the Veilid node (called once at app startup).
///
/// Starts the real Veilid node, attaches to the P2P network, creates
/// a routing context, and stores all handles in `AppState`. Returns the
/// `VeilidUpdate` receiver for the dispatch loop.
///
/// The node lives for the entire app lifetime — user login/logout does NOT
/// restart the node. Only `shutdown_app()` (on app exit) shuts it down.
pub async fn initialize_node(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<mpsc::Receiver<VeilidUpdate>, String> {
    // Determine storage directory inside the Tauri app data dir
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    let storage_dir = app_data_dir.join("veilid");
    std::fs::create_dir_all(&storage_dir)
        .map_err(|e| format!("failed to create veilid storage dir: {e}"))?;

    let config = rekindle_protocol::node::NodeConfig {
        storage_dir: storage_dir.to_string_lossy().into_owned(),
        app_namespace: "rekindle".into(),
        qualifier: "rekindle".into(),
    };

    // Start the real Veilid node (api_startup + attach + routing_context)
    let mut node = rekindle_protocol::RekindleNode::start(config)
        .await
        .map_err(|e| format!("failed to start veilid node: {e}"))?;

    // Take the VeilidUpdate receiver before storing the node's pieces
    let update_rx = node.take_update_receiver();

    // Clone Arc-based handles before storing
    let api = node.api().clone();
    let routing_context = node.routing_context().clone();

    // Store NodeHandle in AppState
    // is_attached starts false — the dispatch loop will set it to true
    // when the first Attachment event with is_attached() arrives.
    let node_handle = NodeHandle {
        attachment_state: "detached".to_string(),
        is_attached: false,
        public_internet_ready: false,
        api: api.clone(),
        routing_context: routing_context.clone(),
        route_blob: None,
        profile_dht_key: None,
        profile_owner_keypair: None,
        friend_list_dht_key: None,
        friend_list_owner_keypair: None,
        account_dht_key: None,
        mailbox_dht_key: None,
    };
    *state.node.write() = Some(node_handle);

    // Create and store DHTManager
    let dht_handle = DHTManagerHandle::new(routing_context);
    *state.dht_manager.write() = Some(dht_handle);

    // Create and store RoutingManager (route allocation is deferred to
    // spawn_dht_publish() which waits for the network to be ready first)
    let routing_manager = rekindle_protocol::routing::RoutingManager::new(
        api,
        rekindle_protocol::routing::SafetyMode::default(),
    );
    *state.routing_manager.write() = Some(RoutingManagerHandle {
        manager: routing_manager,
    });

    tracing::info!("rekindle node started and attached");
    Ok(update_rx)
}

/// Clean up user-specific state on logout without shutting down the Veilid node.
///
/// The node stays alive for the entire app lifetime. This function:
/// 1. Aborts user-specific background tasks (sync, game detection, DHT publish)
/// 2. Closes all tracked DHT records
/// 3. Releases the private route
/// 4. Clears user-specific mappings from the DHT manager (but keeps the manager alive)
/// 5. Clears identity, friends, communities, signal manager
///
/// Does NOT call `api.shutdown()` — the node continues running for re-login.
pub async fn logout_cleanup(app_handle: Option<&tauri::AppHandle>, state: &AppState) {
    // 0. Shut down voice if active
    crate::services::voice::shutdown::shutdown_voice(
        state,
        &crate::services::voice::shutdown::VoiceShutdownOpts::FULL,
    )
    .await;

    // Shut down idle service
    {
        let tx = state.idle_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }
    *state.pre_away_status.write() = None;

    // Shut down heartbeat service
    {
        let tx = state.heartbeat_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    // 1. Abort user-specific background tasks
    {
        let mut handles = state.background_handles.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    // 2. Close ALL open DHT records tracked during this session
    {
        let rc_and_keys = {
            let node = state.node.read();
            let rc = node.as_ref().map(|nh| nh.routing_context.clone());
            let keys: Vec<String> = {
                let dht_mgr = state.dht_manager.read();
                dht_mgr
                    .as_ref()
                    .map(|mgr| mgr.open_records.iter().cloned().collect())
                    .unwrap_or_default()
            };
            rc.map(|rc| (rc, keys))
        };
        if let Some((rc, keys)) = rc_and_keys {
            tracing::debug!(count = keys.len(), "closing open DHT records for logout");
            for key_str in &keys {
                if let Ok(record_key) = key_str.parse::<veilid_core::RecordKey>() {
                    if let Err(e) = rc.close_dht_record(record_key).await {
                        tracing::trace!(key = %key_str, error = %e, "close DHT record on logout");
                    }
                }
            }
        }
    }

    // 3. Release private route
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release private route during logout");
            }
        }
    }

    // 4. Clear user-specific state from DHT manager (keep manager alive for re-login)
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.dht_key_to_friend.clear();
            mgr.conversation_key_to_friend.clear();
            mgr.open_records.clear();
            mgr.manager.route_cache.clear();
            mgr.manager.imported_routes.clear();
            mgr.manager.route_id_to_pubkey.clear();
            mgr.manager.profile_key = None;
            mgr.manager.friend_list_key = None;
        }
    }

    // 5. Clear user-specific data from NodeHandle (keep node alive)
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            nh.route_blob = None;
            nh.profile_dht_key = None;
            nh.profile_owner_keypair = None;
            nh.friend_list_dht_key = None;
            nh.friend_list_owner_keypair = None;
            nh.account_dht_key = None;
            nh.mailbox_dht_key = None;
        }
    }

    // Notify the frontend that the route is gone
    if let Some(ah) = app_handle {
        emit_network_status(ah, state);
    }

    // 6. Clear identity/friends/communities/signal
    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();
    *state.signal_manager.lock() = None;
    *state.identity_secret.lock() = None;

    // 7. Clear community-specific state
    state.mek_cache.lock().clear();

    // 8. Stop coordinator services
    {
        let services = std::mem::take(&mut *state.coordinator_services.write());
        for (_cid, handle) in services {
            handle.stop().await;
        }
    }

    // NOTE: Do NOT reset network_ready_tx here. The Veilid node is still alive
    // and attached — the network IS ready. Resetting to false would cause the
    // next login's spawn_dht_publish() to time out waiting for a readiness signal
    // that never arrives (no new Attachment event fires when the node is already attached).

    tracing::info!("logout cleanup complete — node still running");
}

/// Shutdown the Veilid node (called only on app exit).
///
/// Follows the veilid-server shutdown ordering:
/// 1. Signal dispatch loop shutdown
/// 2. Close remaining DHT records
/// 3. Release private route and clear managers
/// 4. `api.shutdown().await`
pub async fn shutdown_app(state: &AppState) {
    // 1. Abort all remaining background tasks
    {
        let mut handles = state.background_handles.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    // 2. Close ALL open DHT records tracked during this session
    {
        let rc_and_keys = {
            let node = state.node.read();
            let rc = node.as_ref().map(|nh| nh.routing_context.clone());
            let keys: Vec<String> = {
                let dht_mgr = state.dht_manager.read();
                dht_mgr
                    .as_ref()
                    .map(|mgr| mgr.open_records.iter().cloned().collect())
                    .unwrap_or_default()
            };
            rc.map(|rc| (rc, keys))
        };
        if let Some((rc, keys)) = rc_and_keys {
            tracing::debug!(count = keys.len(), "closing open DHT records for app exit");
            for key_str in &keys {
                if let Ok(record_key) = key_str.parse::<veilid_core::RecordKey>() {
                    if let Err(e) = rc.close_dht_record(record_key).await {
                        tracing::trace!(key = %key_str, error = %e, "close DHT record on app exit");
                    }
                }
            }
        }
    }

    // 3. Release private route and clear managers
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release private route during app exit");
            }
        }
        *rm = None;
    }
    *state.dht_manager.write() = None;

    // 4. Shutdown the Veilid API
    let api = {
        let mut node = state.node.write();
        node.take().map(|nh| nh.api)
    };
    if let Some(api) = api {
        api.shutdown().await;
    }

    tracing::info!("veilid node shut down");
}
