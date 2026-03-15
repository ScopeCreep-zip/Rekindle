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
    tracing::info!(msg_len = message.len(), "app_message received");

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

    // Signature is valid — auto-add sender to known_members so we accept
    // their messages immediately (don't wait for next presence_poll_tick).
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.known_members.insert(signed.sender_pseudonym.clone());

            // Update last_seen for TTL-based eviction
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
    // Skip forwarding for private control payloads (targeted at specific member, not broadcast).
    let is_private = is_private_control_payload(&signed.envelope_bytes);
    if signed.ttl > 0 && !is_private {
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

/// Check if a control payload is private (targeted at a specific member, not for broadcast).
/// Private payloads should NOT be forwarded via gossip — they contain key material
/// or data intended only for the direct recipient.
fn is_private_control_payload(envelope_bytes: &[u8]) -> bool {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
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
            sequence,
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
                sequence,
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
            game_info,
            route_blob,
        } => {
            use crate::channels::CommunityEvent;

            // Update route_blob in gossip overlay so we can reach this member.
            // Also add to known_members and peers so messages are accepted and
            // we can send to them immediately (not waiting for next presence poll).
            if let Some(ref blob) = route_blob {
                if !blob.is_empty() {
                    let mut communities = state.communities.write();
                    if let Some(cs) = communities.get_mut(&community_id) {
                        cs.known_members.insert(pseudonym_key.clone());
                        if let Some(ref mut gossip) = cs.gossip {
                            let member = crate::state::OnlineMember {
                                route_blob: blob.clone(),
                                last_seen: rekindle_utils::timestamp_secs(),
                            };
                            gossip.online_members.insert(pseudonym_key.clone(), member.clone());
                            gossip.peers.insert(pseudonym_key.clone(), member);
                        }
                    }
                }
            }

            let (game_name, game_id, elapsed_seconds, server_address) =
                if let Some(gi) = game_info {
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
            handle_relayed_control(
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

/// Process a MemberJoinRequest as a peer admin.
///
/// Any member with `registry_owner_keypair` can add a self-registered joiner
/// to the member index, then broadcast `MemberJoined` via gossip.
fn handle_peer_assisted_join(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    claimed_subkey_index: Option<u32>,
    route_blob: Option<Vec<u8>>,
) {
    use rekindle_protocol::dht::community::envelope::{ControlPayload, CommunityEnvelope};

    let has_registry_kp = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|cs| cs.registry_owner_keypair.is_some())
    };

    if !has_registry_kp {
        return;
    }

    let state = state.clone();
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    let dn = display_name.to_string();
    tokio::spawn(async move {
        match crate::services::coordinator::state_manager::add_member_to_registry(
            &state, &cid, &pk, &dn, claimed_subkey_index,
        )
        .await
        {
            Ok(idx) => {
                tracing::info!(
                    community = %cid,
                    pseudonym = %pk,
                    subkey_index = idx,
                    "peer-assisted join: added member to registry"
                );
                let joined = CommunityEnvelope::Control(ControlPayload::MemberJoined {
                    pseudonym_key: pk.clone(),
                    display_name: dn.clone(),
                    role_ids: vec![0, 1],
                    route_blob: route_blob.clone(),
                });
                crate::services::coordinator::state_manager::broadcast_via_gossip(
                    &state, &cid, &joined,
                );
                crate::services::coordinator::state_manager::emit_local_member_joined(
                    &state, &cid, &pk, &dn,
                );
            }
            Err(e) => {
                tracing::warn!(
                    community = %cid,
                    pseudonym = %pk,
                    error = %e,
                    "peer-assisted join: failed to add member to registry"
                );
            }
        }
    });
}

/// Handle onboarding answers from a member — any admin with `registry_owner_keypair`
/// can evaluate answers and assign roles.
fn handle_onboarding_answers(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    answers: &[rekindle_protocol::dht::community::envelope::OnboardingAnswer],
) {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

    // Only process if we have registry_owner_keypair (admin capability)
    let has_admin_kp = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|cs| cs.registry_owner_keypair.is_some())
    };
    if !has_admin_kp {
        return;
    }

    let state = state.clone();
    let cid = community_id.to_string();
    let sender = sender_pseudonym.to_string();
    let answers = answers.to_vec();
    tokio::spawn(async move {
        match crate::services::coordinator::onboarding::process_answers(&state, &cid, &answers)
            .await
        {
            Ok(role_ids) if !role_ids.is_empty() => {
                // Assign roles via DHT registry
                for &rid in &role_ids {
                    if let Err(e) =
                        crate::services::coordinator::state_manager::persist_role_assignment_pub(
                            &state, &cid, &sender, rid, true,
                        )
                        .await
                    {
                        tracing::warn!(
                            community = %cid, pseudonym = %sender, role_id = rid,
                            error = %e, "failed to assign onboarding role"
                        );
                    }
                }
                // Mark onboarding_complete in member registry
                if let Err(e) = crate::services::coordinator::state_manager::set_onboarding_complete_pub(
                    &state, &cid, &sender,
                ).await {
                    tracing::warn!(
                        community = %cid, pseudonym = %sender, error = %e,
                        "failed to set onboarding_complete in registry"
                    );
                }
                // Broadcast OnboardingComplete via gossip
                let notification = ControlPayload::OnboardingComplete {
                    pseudonym_key: sender.clone(),
                    role_ids: role_ids.clone(),
                };
                let envelope = CommunityEnvelope::Control(notification);
                crate::services::coordinator::state_manager::broadcast_via_gossip(
                    &state, &cid, &envelope,
                );
                tracing::info!(
                    community = %cid, pseudonym = %sender, roles = ?role_ids,
                    "onboarding complete — roles assigned via gossip"
                );
            }
            Ok(_) => {} // no roles to assign
            Err(e) => {
                tracing::warn!(
                    community = %cid, pseudonym = %sender, error = %e,
                    "failed to process onboarding answers"
                );
            }
        }
    });
}

/// Handle a relayed control payload.
async fn handle_relayed_control(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        // MemberJoinRequest — any admin with registry_owner_keypair can process this.
        // Self-service joiners broadcast this after writing SMPL presence to request
        // formal member index registration.
        ControlPayload::MemberJoinRequest {
            pseudonym_key,
            display_name,
            claimed_subkey_index,
            route_blob,
            ..
        } => {
            handle_peer_assisted_join(
                state,
                community_id,
                &pseudonym_key,
                &display_name,
                claimed_subkey_index,
                route_blob,
            );
        }
        ControlPayload::MemberJoined {
            pseudonym_key,
            display_name,
            role_ids,
            route_blob,
        } => {
            // Add to known_members cache
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.known_members.insert(pseudonym_key.clone());
                }
            }

            // Add joiner to gossip overlay if route_blob is available
            if let Some(ref blob) = route_blob {
                if !blob.is_empty() {
                    let mut communities = state.communities.write();
                    if let Some(cs) = communities.get_mut(community_id) {
                        if cs.gossip.is_none() {
                            cs.gossip = Some(crate::state::GossipOverlay::default());
                        }
                        if let Some(ref mut gossip) = cs.gossip {
                            let member = crate::state::OnlineMember {
                                route_blob: blob.clone(),
                                last_seen: rekindle_utils::timestamp_secs(),
                            };
                            gossip.online_members.insert(pseudonym_key.clone(), member.clone());
                            gossip.peers.insert(pseudonym_key.clone(), member);
                        }
                    }
                }
            }

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
        ControlPayload::MemberRemoved { pseudonym_key }
        | ControlPayload::MemberLeave { pseudonym_key } => {
            // Remove from SQLite (covers both kick/ban removal and voluntary leave)
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let pk = pseudonym_key.clone();
            crate::db_helpers::db_fire(pool.inner(), "persist MemberRemoved/Leave", move |conn| {
                conn.execute(
                    "DELETE FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![owner_key, cid, pk],
                )?;
                Ok(())
            });

            // Remove from gossip overlay's online members + known_members cache
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.known_members.remove(&pseudonym_key);
                    if let Some(ref mut gossip) = cs.gossip {
                        gossip.online_members.remove(&pseudonym_key);
                        gossip.peers.remove(&pseudonym_key);
                    }
                }
            }

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
            // Persist timeout to SQLite
            let ok = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let tp = pseudonym_key.clone();
            db_fire(pool, "relayed_member_timed_out", move |conn| {
                conn.execute(
                    "UPDATE community_members SET timeout_until = ?1 \
                     WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                    rusqlite::params![timeout_until, ok, cid, tp],
                )?;
                Ok(())
            });
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
            handle_roles_changed(app_handle, state, community_id, &roles);
        }
        ControlPayload::MemberRolesChanged {
            pseudonym_key,
            role_ids,
        } => {
            handle_member_roles_changed(app_handle, state, community_id, &pseudonym_key, &role_ids);
        }
        // JoinAccepted — coordinator sent us community data + MEK after our join
        ControlPayload::JoinAccepted {
            mek_encrypted,
            mek_generation,
            channels,
            categories: _,
            role_ids,
            roles,
            members,
            member_registry_key,
            channel_log_keypairs: _,
            slot_index,
            wrapped_slot_seed,
            wrapped_slot_keypair,
        } => {
            handle_join_accepted(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                &JoinAcceptedData {
                    mek_wire_bytes: &mek_encrypted,
                    mek_generation,
                    channels: &channels,
                    role_ids: &role_ids,
                    roles: &roles,
                    members: &members,
                    member_registry_key: member_registry_key.as_deref(),
                    slot_index,
                    wrapped_slot_seed: wrapped_slot_seed.as_deref(),
                    wrapped_slot_keypair: wrapped_slot_keypair.as_deref(),
                },
            )
            .await;
        }
        // JoinRejected — peer denied our join request
        ControlPayload::JoinRejected { reason } => {
            tracing::warn!(
                community = %community_id,
                reason = %reason,
                "join request rejected by peer"
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
                pool,
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
    pool: &DbPool,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::ChannelsUpdated { channels, categories } => {
            handle_channels_updated(app_handle, state, community_id, &channels, &categories);
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
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let ch = channel_id.clone();
            let mid = message_id.clone();
            let pb = pinned_by.clone();
            let now = rekindle_utils::timestamp_secs();
            crate::db_helpers::db_fire(pool, "persist pin", move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO channel_pins (owner_key, community_id, channel_id, message_id, pinned_by, pinned_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![owner_key, cid, ch, mid, pb, now],
                )?;
                Ok(())
            });
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
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let ch = channel_id.clone();
            let mid = message_id.clone();
            crate::db_helpers::db_fire(pool, "remove pin", move |conn| {
                conn.execute(
                    "DELETE FROM channel_pins WHERE owner_key = ?1 AND community_id = ?2 \
                     AND channel_id = ?3 AND message_id = ?4",
                    rusqlite::params![owner_key, cid, ch, mid],
                )?;
                Ok(())
            });
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
            code_hash,
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
                    code_hash,
                    created_by,
                    max_uses,
                    uses,
                    expires_at,
                    created_at,
                },
            );
        }
        ControlPayload::InviteRevoked { code_hash } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::InviteRevoked {
                    community_id: community_id.to_string(),
                    code_hash,
                },
            );
        }
        ControlPayload::InviteUsed {
            code_hash,
            new_use_count,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::InviteUsed {
                    community_id: community_id.to_string(),
                    code_hash,
                    new_use_count,
                },
            );
        }
        // Events, threads, game servers, and remaining payloads
        other => {
            handle_control_events_and_threads(app_handle, state, pool, community_id, sender_pseudonym, other);
        }
    }
}

/// Handle event, thread, and game server control payloads with SQLite persistence.
fn handle_control_events_and_threads(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::EventCreated { event } => {
            if let Ok(dto) = serde_json::from_value::<crate::channels::community_channel::EventInfoDto>(event) {
                let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
                let cid = community_id.to_string();
                let d = dto.clone();
                crate::db_helpers::db_fire(pool, "persist event", move |conn| {
                    conn.execute(
                        "INSERT OR REPLACE INTO community_events \
                         (owner_key, community_id, id, title, description, creator_pseudonym, \
                          start_time, end_time, channel_id, max_attendees, created_at, status) \
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                        rusqlite::params![
                            owner_key, cid, d.id, d.title, d.description,
                            d.creator_pseudonym, d.start_time, d.end_time,
                            d.channel_id, d.max_attendees, d.created_at, d.status,
                        ],
                    )?;
                    for rsvp in &d.rsvps {
                        conn.execute(
                            "INSERT OR REPLACE INTO event_rsvps \
                             (owner_key, community_id, event_id, pseudonym_key, status) \
                             VALUES (?1,?2,?3,?4,?5)",
                            rusqlite::params![owner_key, cid, d.id, rsvp.pseudonym_key, rsvp.status],
                        )?;
                    }
                    Ok(())
                });
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
                let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
                let cid = community_id.to_string();
                let d = dto.clone();
                crate::db_helpers::db_fire(pool, "update event", move |conn| {
                    conn.execute(
                        "INSERT OR REPLACE INTO community_events \
                         (owner_key, community_id, id, title, description, creator_pseudonym, \
                          start_time, end_time, channel_id, max_attendees, created_at, status) \
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                        rusqlite::params![
                            owner_key, cid, d.id, d.title, d.description,
                            d.creator_pseudonym, d.start_time, d.end_time,
                            d.channel_id, d.max_attendees, d.created_at, d.status,
                        ],
                    )?;
                    Ok(())
                });
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
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let eid = event_id.clone();
            crate::db_helpers::db_fire(pool, "delete event", move |conn| {
                conn.execute(
                    "DELETE FROM community_events WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
                    rusqlite::params![owner_key, cid, eid],
                )?;
                conn.execute(
                    "DELETE FROM event_rsvps WHERE owner_key = ?1 AND community_id = ?2 AND event_id = ?3",
                    rusqlite::params![owner_key, cid, eid],
                )?;
                Ok(())
            });
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
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let eid = event_id.clone();
            let pk = pseudonym_key.clone();
            let st = status.clone();
            crate::db_helpers::db_fire(pool, "persist rsvp", move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO event_rsvps \
                     (owner_key, community_id, event_id, pseudonym_key, status) \
                     VALUES (?1,?2,?3,?4,?5)",
                    rusqlite::params![owner_key, cid, eid, pk, st],
                )?;
                Ok(())
            });
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
                let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
                let cid = community_id.to_string();
                let d = dto.clone();
                crate::db_helpers::db_fire(pool, "persist thread", move |conn| {
                    conn.execute(
                        "INSERT OR REPLACE INTO community_threads \
                         (owner_key, community_id, id, channel_id, name, starter_message_id, \
                          creator_pseudonym, created_at, archived, auto_archive_seconds, \
                          last_message_at, message_count) \
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                        rusqlite::params![
                            owner_key, cid, d.id, d.channel_id, d.name,
                            d.starter_message_id, d.creator_pseudonym, d.created_at,
                            i32::from(d.archived), d.auto_archive_seconds,
                            d.last_message_at, d.message_count,
                        ],
                    )?;
                    Ok(())
                });
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
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let tid = thread_id.clone();
            let arch = archived;
            crate::db_helpers::db_fire(pool, "update thread archived", move |conn| {
                conn.execute(
                    "UPDATE community_threads SET archived = ?1 \
                     WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
                    rusqlite::params![i32::from(arch), owner_key, cid, tid],
                )?;
                Ok(())
            });
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
                let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
                let cid = community_id.to_string();
                let d = dto.clone();
                crate::db_helpers::db_fire(pool, "persist game server", move |conn| {
                    conn.execute(
                        "INSERT OR REPLACE INTO game_servers \
                         (owner_key, community_id, id, game_id, label, address, added_by, created_at) \
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                        rusqlite::params![
                            owner_key, cid, d.id, d.game_id, d.label,
                            d.address, d.added_by, d.created_at,
                        ],
                    )?;
                    Ok(())
                });
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
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let sid = server_id.clone();
            crate::db_helpers::db_fire(pool, "remove game server", move |conn| {
                conn.execute(
                    "DELETE FROM game_servers WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
                    rusqlite::params![owner_key, cid, sid],
                )?;
                Ok(())
            });
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::GameServerRemoved {
                    community_id: community_id.to_string(),
                    server_id,
                },
            );
        }
        ControlPayload::MEKRotated { new_generation } => {
            // Fetch the new MEK from the DHT vault
            let app = app_handle.clone();
            let state_clone = state.clone();
            let cid = community_id.to_string();
            tokio::spawn(async move {
                fetch_mek_from_dht(&app, &state_clone, &cid).await;
            });
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
        // Onboarding answers — any admin with registry_owner_keypair processes them
        ControlPayload::SubmitOnboardingAnswers { ref answers } => {
            handle_onboarding_answers(state, community_id, sender_pseudonym, answers);
        }
        // Onboarding complete — member's roles were assigned after answering
        ControlPayload::OnboardingComplete {
            ref pseudonym_key,
            ref role_ids,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::OnboardingComplete {
                    community_id: community_id.to_string(),
                    pseudonym_key: pseudonym_key.clone(),
                    role_ids: role_ids.clone(),
                },
            );
        }
        // Threads, thread messages, events, system, and remaining payloads
        other => {
            handle_control_threads_and_misc(app_handle, state, pool, community_id, sender_pseudonym, other);
        }
    }
}

/// Handle thread messages, event reminders, system messages, and remaining control payloads.
fn handle_control_threads_and_misc(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
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
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let tid = thread_id.clone();
            let mid = message_id.clone();
            let sp = sender_pseudonym.clone();
            let b = body.clone();
            let ts = timestamp;
            let rid = reply_to_id.clone();
            crate::db_helpers::db_fire(pool, "persist thread message", move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO thread_messages \
                     (owner_key, community_id, thread_id, message_id, sender_pseudonym, body, timestamp, reply_to_id) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    rusqlite::params![owner_key, cid, tid, mid, sp, b, ts, rid],
                )?;
                // Bump thread message_count and last_message_at
                conn.execute(
                    "UPDATE community_threads SET message_count = message_count + 1, last_message_at = ?1 \
                     WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
                    rusqlite::params![ts, owner_key, cid, tid],
                )?;
                Ok(())
            });
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
            // Clear pending sync — response received
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.pending_syncs.remove(&channel_id);
                }
            }
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

        // ── Voice signaling (handled in voice::signaling module) ──
        ControlPayload::VoiceJoin { .. }
        | ControlPayload::VoiceLeave { .. }
        | ControlPayload::VoiceModeSwitch { .. }
        | ControlPayload::VoiceMute { .. }
        | ControlPayload::VoiceDeafen { .. }
        | ControlPayload::VoiceRoster { .. } => {
            super::voice::signaling::handle_voice_signaling(
                app_handle, state, community_id, sender_pseudonym, payload,
            );
        }


        // ── Moderation payloads (paper-shredded: every member applies these) ──
        // Envelope signature already verified by gossip layer. Source permission
        // was checked at the originating command handler (require_permission).
        other => {
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            handle_gossip_moderation(app_handle, state, &pool, community_id, sender_pseudonym, other);
        }
    }
}

/// Handle moderation and structural control payloads received via gossip mesh.
///
/// In the paper-shredded model, any member with proper permissions can initiate
/// moderation actions. Every member who receives these via gossip applies them
/// locally (update in-memory state, persist to SQLite, emit frontend events).
fn handle_gossip_moderation(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use crate::db_helpers::db_fire;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    let Ok(owner_key) = crate::state_helpers::current_owner_key(state) else {
        return;
    };

    // Verify the sender has the required permission for this moderation action.
    if !check_gossip_moderation_permission(state, community_id, sender_pseudonym, &payload) {
        return;
    }

    match payload {
        // ── Kick: remove the target from local state ──
        ControlPayload::Kick { target_pseudonym } => {
            let my_pseudonym = {
                let communities = state.communities.read();
                communities
                    .get(community_id)
                    .and_then(|cs| cs.my_pseudonym_key.clone())
            };

            if my_pseudonym.as_deref() == Some(&target_pseudonym) {
                // We were kicked
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::Kicked {
                        community_id: community_id.to_string(),
                    },
                );
            } else {
                // Someone else was kicked — remove from local state + SQLite
                {
                    let mut communities = state.communities.write();
                    if let Some(cs) = communities.get_mut(community_id) {
                        cs.known_members.remove(&target_pseudonym);
                        if let Some(ref mut gossip) = cs.gossip {
                            gossip.online_members.remove(&target_pseudonym);
                            gossip.peers.remove(&target_pseudonym);
                        }
                    }
                }
                let ok = owner_key.clone();
                let cid = community_id.to_string();
                let tp = target_pseudonym.clone();
                db_fire(pool, "kick_member_remove", move |conn| {
                    conn.execute(
                        "DELETE FROM community_members WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                        rusqlite::params![ok, cid, tp],
                    )?;
                    Ok(())
                });
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::MemberRemoved {
                        community_id: community_id.to_string(),
                        pseudonym_key: target_pseudonym,
                    },
                );
            }
        }

        // ── Ban: remove target from state + SQLite ──
        ControlPayload::Ban { target_pseudonym, .. } => {
            let my_pseudonym = {
                let communities = state.communities.read();
                communities
                    .get(community_id)
                    .and_then(|cs| cs.my_pseudonym_key.clone())
            };

            if my_pseudonym.as_deref() == Some(&target_pseudonym) {
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::Kicked {
                        community_id: community_id.to_string(),
                    },
                );
            } else {
                {
                    let mut communities = state.communities.write();
                    if let Some(cs) = communities.get_mut(community_id) {
                        cs.known_members.remove(&target_pseudonym);
                        if let Some(ref mut gossip) = cs.gossip {
                            gossip.online_members.remove(&target_pseudonym);
                            gossip.peers.remove(&target_pseudonym);
                        }
                    }
                }
                let ok = owner_key.clone();
                let cid = community_id.to_string();
                let tp = target_pseudonym.clone();
                db_fire(pool, "ban_member_remove", move |conn| {
                    conn.execute(
                        "DELETE FROM community_members WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                        rusqlite::params![ok, cid, tp],
                    )?;
                    Ok(())
                });
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::MemberRemoved {
                        community_id: community_id.to_string(),
                        pseudonym_key: target_pseudonym,
                    },
                );
            }
        }

        // ── Unban: no local state change needed (ban list is in DHT manifest) ──
        ControlPayload::Unban { .. } => {}

        // ── Timeout: compute timeout_until, persist to SQLite, emit ──
        ControlPayload::TimeoutMember {
            target_pseudonym,
            duration_seconds,
            ..
        } => {
            let timeout_until = rekindle_utils::timestamp_secs() + duration_seconds;
            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let tp = target_pseudonym.clone();
            db_fire(pool, "timeout_member", move |conn| {
                conn.execute(
                    "UPDATE community_members SET timeout_until = ?1 \
                     WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                    rusqlite::params![timeout_until, ok, cid, tp],
                )?;
                Ok(())
            });
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberTimedOut {
                    community_id: community_id.to_string(),
                    pseudonym_key: target_pseudonym,
                    timeout_until: Some(timeout_until),
                },
            );
        }

        // ── Remove timeout: clear in SQLite, emit ──
        ControlPayload::RemoveTimeout { target_pseudonym } => {
            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let tp = target_pseudonym.clone();
            db_fire(pool, "remove_timeout", move |conn| {
                conn.execute(
                    "UPDATE community_members SET timeout_until = NULL \
                     WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                    rusqlite::params![ok, cid, tp],
                )?;
                Ok(())
            });
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberTimedOut {
                    community_id: community_id.to_string(),
                    pseudonym_key: target_pseudonym,
                    timeout_until: None,
                },
            );
        }

        // ── Role assignment: update local state + SQLite + emit ──
        ControlPayload::AssignRole {
            target_pseudonym,
            role_id,
            ..
        } => {
            let my_pseudonym = {
                let communities = state.communities.read();
                communities
                    .get(community_id)
                    .and_then(|cs| cs.my_pseudonym_key.clone())
            };
            let is_self = my_pseudonym.as_deref() == Some(&target_pseudonym);
            let self_roles_json = if is_self {
                let mut communities = state.communities.write();
                communities.get_mut(community_id).map(|cs| {
                    if !cs.my_role_ids.contains(&role_id) {
                        cs.my_role_ids.push(role_id);
                    }
                    serde_json::to_string(&cs.my_role_ids).unwrap_or_default()
                })
            } else {
                None
            };

            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let tp = target_pseudonym.clone();
            db_fire(pool, "assign_role_persist", move |conn| {
                let current: Option<String> = conn.query_row(
                    "SELECT role_ids FROM community_members \
                     WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                    rusqlite::params![ok, cid, tp],
                    |row| row.get(0),
                ).ok();
                if let Some(json) = current {
                    let mut ids: Vec<u32> = serde_json::from_str(&json).unwrap_or_default();
                    if !ids.contains(&role_id) { ids.push(role_id); }
                    conn.execute(
                        "UPDATE community_members SET role_ids = ?1 \
                         WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                        rusqlite::params![serde_json::to_string(&ids).unwrap_or_default(), ok, cid, tp],
                    )?;
                }
                if let Some(ref srj) = self_roles_json {
                    conn.execute(
                        "UPDATE communities SET my_role_ids = ?1 WHERE owner_key = ?2 AND id = ?3",
                        rusqlite::params![srj, ok, cid],
                    )?;
                }
                Ok(())
            });

            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberRolesChanged {
                    community_id: community_id.to_string(),
                    pseudonym_key: target_pseudonym,
                    role_ids: vec![role_id],
                },
            );
        }

        ControlPayload::UnassignRole {
            target_pseudonym,
            role_id,
            ..
        } => {
            let my_pseudonym = {
                let communities = state.communities.read();
                communities
                    .get(community_id)
                    .and_then(|cs| cs.my_pseudonym_key.clone())
            };
            let is_self = my_pseudonym.as_deref() == Some(&target_pseudonym);
            let self_roles_json = if is_self {
                let mut communities = state.communities.write();
                communities.get_mut(community_id).map(|cs| {
                    cs.my_role_ids.retain(|&r| r != role_id);
                    serde_json::to_string(&cs.my_role_ids).unwrap_or_default()
                })
            } else {
                None
            };

            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let tp = target_pseudonym.clone();
            db_fire(pool, "unassign_role_persist", move |conn| {
                let current: Option<String> = conn.query_row(
                    "SELECT role_ids FROM community_members \
                     WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                    rusqlite::params![ok, cid, tp],
                    |row| row.get(0),
                ).ok();
                if let Some(json) = current {
                    let mut ids: Vec<u32> = serde_json::from_str(&json).unwrap_or_default();
                    ids.retain(|&r| r != role_id);
                    conn.execute(
                        "UPDATE community_members SET role_ids = ?1 \
                         WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                        rusqlite::params![serde_json::to_string(&ids).unwrap_or_default(), ok, cid, tp],
                    )?;
                }
                if let Some(ref srj) = self_roles_json {
                    conn.execute(
                        "UPDATE communities SET my_role_ids = ?1 WHERE owner_key = ?2 AND id = ?3",
                        rusqlite::params![srj, ok, cid],
                    )?;
                }
                Ok(())
            });

            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::MemberRolesChanged {
                    community_id: community_id.to_string(),
                    pseudonym_key: target_pseudonym,
                    role_ids: vec![],
                },
            );
        }

        // Structural payloads (roles, overwrites, community metadata) — delegated
        // to avoid exceeding the function line limit.
        other => {
            handle_gossip_structural(app_handle, state, pool, community_id, &owner_key, other);
        }
    }
}

/// Handle structural gossip payloads: role edits, channel overwrites, community metadata.
///
/// Split from `handle_gossip_moderation` to keep functions within the 300-line limit.
fn handle_gossip_structural(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    owner_key: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use crate::db_helpers::db_fire;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        // ── Role editing: update local state + SQLite + emit ──
        ControlPayload::EditRole {
            role_id,
            name,
            color,
            permissions,
            position,
            hoist,
            mentionable,
        } => {
            let updated_roles = {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    if let Some(r) = cs.roles.iter_mut().find(|r| r.id == role_id) {
                        if let Some(ref n) = name { r.name.clone_from(n); }
                        if let Some(c) = color { r.color = c; }
                        if let Some(p) = permissions { r.permissions = p; }
                        if let Some(pos) = position { r.position = pos; }
                        if let Some(h) = hoist { r.hoist = h; }
                        if let Some(m) = mentionable { r.mentionable = m; }
                    }
                    Some(cs.roles.clone())
                } else {
                    None
                }
            };
            // Persist to SQLite community_roles
            let ok = owner_key.to_string();
            let cid = community_id.to_string();
            db_fire(pool, "edit_role_persist", move |conn| {
                let mut sets = Vec::new();
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                if let Some(ref n) = name { sets.push("name = ?"); params.push(Box::new(n.clone())); }
                if let Some(c) = color { sets.push("color = ?"); params.push(Box::new(i64::from(c))); }
                if let Some(p) = permissions { sets.push("permissions = ?"); params.push(Box::new(p.cast_signed())); }
                if let Some(pos) = position { sets.push("position = ?"); params.push(Box::new(i64::from(pos))); }
                if let Some(h) = hoist { sets.push("hoist = ?"); params.push(Box::new(i64::from(h))); }
                if let Some(m) = mentionable { sets.push("mentionable = ?"); params.push(Box::new(i64::from(m))); }
                if !sets.is_empty() {
                    params.push(Box::new(ok));
                    params.push(Box::new(cid));
                    params.push(Box::new(i64::from(role_id)));
                    let sql = format!(
                        "UPDATE community_roles SET {} WHERE owner_key = ? AND community_id = ? AND role_id = ?",
                        sets.join(", ")
                    );
                    let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(std::convert::AsRef::as_ref).collect();
                    conn.execute(&sql, refs.as_slice())?;
                }
                Ok(())
            });
            if let Some(roles) = updated_roles {
                let role_dtos: Vec<crate::channels::community_channel::RoleDto> = roles
                    .iter()
                    .map(|r| crate::channels::community_channel::RoleDto {
                        id: r.id,
                        name: r.name.clone(),
                        color: r.color,
                        permissions: r.permissions,
                        position: r.position,
                        hoist: r.hoist,
                        mentionable: r.mentionable,
                    })
                    .collect();
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::RolesChanged {
                        community_id: community_id.to_string(),
                        roles: role_dtos,
                    },
                );
            }
        }

        // ── Channel/role overwrites ──
        ControlPayload::SetChannelOverwrite { channel_id, .. }
        | ControlPayload::DeleteChannelOverwrite { channel_id, .. } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::ChannelOverwriteChanged {
                    community_id: community_id.to_string(),
                    channel_id,
                },
            );
        }

        // ── Community metadata update: persist to SQLite ──
        ControlPayload::UpdateCommunity { name, description } => {
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    if let Some(ref n) = name {
                        cs.name.clone_from(n);
                    }
                    if let Some(ref d) = description {
                        cs.description = Some(d.clone());
                    }
                }
            }
            let ok = owner_key.to_string();
            let cid = community_id.to_string();
            let n = name.clone();
            let d = description.clone();
            db_fire(pool, "update_community_persist", move |conn| {
                if let Some(ref name_val) = n {
                    conn.execute(
                        "UPDATE communities SET name = ?1 WHERE owner_key = ?2 AND id = ?3",
                        rusqlite::params![name_val, ok, cid],
                    )?;
                }
                if let Some(ref desc_val) = d {
                    conn.execute(
                        "UPDATE communities SET description = ?1 WHERE owner_key = ?2 AND id = ?3",
                        rusqlite::params![desc_val, ok, cid],
                    )?;
                }
                Ok(())
            });
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::CommunityUpdated {
                    community_id: community_id.to_string(),
                    name,
                    description,
                },
            );
        }

        // Payloads not needing client-side handling
        _ => {
            tracing::trace!(
                community = %community_id,
                "received unhandled gossip control payload"
            );
        }
    }
}

/// Handle `RolesChanged`: update CommunityState.roles + persist to SQLite + emit.
fn handle_roles_changed(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    roles: &[serde_json::Value],
) {
    use crate::channels::CommunityEvent;

    let role_dtos: Vec<crate::channels::community_channel::RoleDto> = roles
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();
    // Update in-memory state
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.roles = role_dtos
                .iter()
                .map(|r| crate::state::RoleDefinition {
                    id: r.id,
                    name: r.name.clone(),
                    color: r.color,
                    permissions: r.permissions,
                    position: r.position,
                    hoist: r.hoist,
                    mentionable: r.mentionable,
                })
                .collect();
        }
    }
    // Persist to SQLite
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    if let Ok(owner_key) = crate::state_helpers::current_owner_key(state) {
        let cid = community_id.to_string();
        let roles_for_db = role_dtos.clone();
        crate::db_helpers::db_fire(&pool, "roles_changed_persist", move |conn| {
            conn.execute(
                "DELETE FROM community_roles WHERE owner_key = ?1 AND community_id = ?2",
                rusqlite::params![owner_key, cid],
            )?;
            for r in &roles_for_db {
                conn.execute(
                    "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        owner_key, cid, r.id, r.name, r.color,
                        r.permissions.cast_signed(), r.position, r.hoist, r.mentionable
                    ],
                )?;
            }
            Ok(())
        });
    }
    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::RolesChanged {
            community_id: community_id.to_string(),
            roles: role_dtos,
        },
    );
}

/// Handle `MemberRolesChanged`: update my_role_ids if self + persist to SQLite + emit.
fn handle_member_roles_changed(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    role_ids: &[u32],
) {
    use crate::channels::CommunityEvent;

    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
    };
    if my_pseudonym.as_deref() == Some(pseudonym_key) && !role_ids.is_empty() {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.my_role_ids = role_ids.to_vec();
        }
    }
    // Persist to SQLite
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    if let Ok(owner_key) = crate::state_helpers::current_owner_key(state) {
        let cid = community_id.to_string();
        let pk = pseudonym_key.to_string();
        let rids = role_ids.to_vec();
        let is_self = my_pseudonym.as_deref() == Some(pseudonym_key);
        crate::db_helpers::db_fire(&pool, "member_roles_changed_persist", move |conn| {
            let json = serde_json::to_string(&rids).unwrap_or_default();
            conn.execute(
                "UPDATE community_members SET role_ids = ?1 \
                 WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                rusqlite::params![json, owner_key, cid, pk],
            )?;
            if is_self {
                conn.execute(
                    "UPDATE communities SET my_role_ids = ?1 WHERE owner_key = ?2 AND id = ?3",
                    rusqlite::params![json, owner_key, cid],
                )?;
            }
            Ok(())
        });
    }
    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::MemberRolesChanged {
            community_id: community_id.to_string(),
            pseudonym_key: pseudonym_key.to_string(),
            role_ids: role_ids.to_vec(),
        },
    );
}

/// Handle `ChannelsUpdated`: update CommunityState.channels/categories + persist + emit.
fn handle_channels_updated(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channels: &[serde_json::Value],
    categories: &[serde_json::Value],
) {
    use crate::channels::CommunityEvent;

    let channel_dtos: Vec<crate::channels::community_channel::ChannelInfoFrontendDto> = channels
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();
    let category_dtos: Vec<crate::channels::community_channel::CategoryInfoFrontendDto> =
        categories
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
    // Update CommunityState (preserve local-only fields like unread_count)
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            let existing: std::collections::HashMap<String, &crate::state::ChannelInfo> =
                cs.channels.iter().map(|c| (c.id.clone(), c)).collect();
            cs.channels = channel_dtos
                .iter()
                .map(|dto| {
                    let prev = existing.get(&dto.id);
                    crate::state::ChannelInfo {
                        id: dto.id.clone(),
                        name: dto.name.clone(),
                        channel_type: dto.channel_type
                            .parse()
                            .unwrap_or(crate::state::ChannelType::Text),
                        unread_count: prev.map_or(0, |p| p.unread_count),
                        category_id: dto.category_id.clone(),
                        topic: dto.topic.clone(),
                        slowmode_seconds: if dto.slowmode_seconds > 0 {
                            Some(dto.slowmode_seconds)
                        } else {
                            None
                        },
                        nsfw: prev.is_some_and(|p| p.nsfw),
                        message_record_key: prev.and_then(|p| p.message_record_key.clone()),
                        mek_generation: prev.map_or(0, |p| p.mek_generation),
                    }
                })
                .collect();
            cs.categories = category_dtos
                .iter()
                .map(|dto| crate::state::CategoryInfo {
                    id: dto.id.clone(),
                    name: dto.name.clone(),
                    sort_order: dto.sort_order,
                })
                .collect();
        }
    }
    // Persist channels + categories to SQLite
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    if let Ok(owner_key) = crate::state_helpers::current_owner_key(state) {
        let cid = community_id.to_string();
        let ch = channel_dtos.clone();
        let cats = category_dtos.clone();
        crate::db_helpers::db_fire(&pool, "channels_updated_persist", move |conn| {
            for dto in &ch {
                conn.execute(
                    "INSERT INTO channels \
                     (owner_key, id, community_id, name, channel_type, sort_order, category_id, topic, slowmode_seconds) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8) \
                     ON CONFLICT(owner_key, id) DO UPDATE SET \
                       name = excluded.name, \
                       channel_type = excluded.channel_type, \
                       category_id = excluded.category_id, \
                       topic = excluded.topic, \
                       slowmode_seconds = excluded.slowmode_seconds",
                    rusqlite::params![
                        owner_key, dto.id, cid, dto.name, dto.channel_type,
                        dto.category_id, dto.topic, dto.slowmode_seconds
                    ],
                )?;
            }
            // Persist categories: delete all for this community, then re-insert
            conn.execute(
                "DELETE FROM community_categories WHERE owner_key = ?1 AND community_id = ?2",
                rusqlite::params![owner_key, cid],
            )?;
            for dto in &cats {
                conn.execute(
                    "INSERT INTO community_categories (owner_key, community_id, id, name, sort_order) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![owner_key, cid, dto.id, dto.name, dto.sort_order],
                )?;
            }
            Ok(())
        });
    }
    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::ChannelsUpdated {
            community_id: community_id.to_string(),
            channels: channel_dtos,
            categories: category_dtos,
        },
    );
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

    // Persist to CommunityState (keypair + subkey index)
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.slot_keypair = Some(slot_kp_str.clone());
            c.my_subkey_index = Some(slot_index);
        }
    }

    // Persist to Stronghold
    let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let ks = ks_handle.lock();
    if let Some(ref keystore) = *ks {
        crate::keystore::persist_slot_keypair(keystore, community_id, &slot_kp_str);
    }

    // Persist subkey_index to SQLite
    {
        let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
        let cid = community_id.to_string();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        let idx = i64::from(slot_index);
        crate::db_helpers::db_fire(pool.inner(), "persist my_subkey_index", move |conn| {
            conn.execute(
                "UPDATE communities SET my_subkey_index = ?1 WHERE owner_key = ?2 AND id = ?3",
                rusqlite::params![idx, owner_key, cid],
            )?;
            Ok(())
        });
    }

    tracing::info!(
        community = %community_id,
        slot_index, segment_index,
        "slot keypair grant accepted and persisted"
    );

    // Trigger an immediate presence write so peers discover us quickly
    // (next scheduled tick may be up to 60s away)
    let state_for_poll = state.clone();
    let cid_for_poll = community_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = crate::services::community::presence_poll_tick_public(
            &state_for_poll,
            &cid_for_poll,
        )
        .await
        {
            tracing::debug!(error = %e, "immediate presence poll after SlotKeypairGrant failed");
        }
    });
}

/// Unwrap slot_seed, persist it, and derive the slot keypair locally.
///
/// This is the coordinator-free path: the member receives the seed and derives
/// their own keypair via `derive_slot_veilid_keypair(seed, slot_index)`.
fn handle_slot_seed_grant(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    slot_index: u32,
    wrapped_slot_seed: &[u8],
) {
    use rekindle_crypto::group::mek_distribution::unwrap_mek;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
    use rekindle_protocol::dht::community::member_registry;

    let Some(secret) = state.identity_secret.lock().as_ref().copied() else {
        tracing::warn!("no identity secret — cannot unwrap slot seed");
        return;
    };
    let my_signing_key = derive_community_pseudonym(&secret, community_id);

    let Ok(sender_bytes) = hex::decode(sender_pseudonym) else {
        tracing::warn!("invalid sender pseudonym hex in slot seed grant");
        return;
    };
    let Ok(sender_pub): Result<[u8; 32], _> = sender_bytes.try_into() else {
        tracing::warn!("sender pseudonym wrong length in slot seed grant");
        return;
    };

    // Unwrap the slot seed
    let seed_bytes = match unwrap_mek(&my_signing_key, &sender_pub, wrapped_slot_seed) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap slot seed");
            return;
        }
    };
    let seed_hex = String::from_utf8_lossy(&seed_bytes).to_string();

    // Derive the slot keypair locally
    let seed_raw = match hex::decode(&seed_hex) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "slot seed is not valid hex");
            return;
        }
    };
    let Ok(seed_array): Result<[u8; 32], _> = seed_raw.try_into() else {
        tracing::warn!("slot seed wrong length (expected 32 bytes)");
        return;
    };
    let slot_kp = match member_registry::derive_slot_veilid_keypair(&seed_array, slot_index) {
        Ok(kp) => kp,
        Err(e) => {
            tracing::warn!(error = %e, slot_index, "failed to derive slot keypair from seed");
            return;
        }
    };
    let slot_kp_str = slot_kp.to_string();

    // Persist slot_seed + derived slot_keypair + subkey_index to CommunityState
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.slot_seed = Some(seed_hex.clone());
            c.slot_keypair = Some(slot_kp_str.clone());
            c.my_subkey_index = Some(slot_index);
        }
    }

    // Persist to Stronghold
    let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let ks = ks_handle.lock();
    if let Some(ref keystore) = *ks {
        crate::keystore::persist_slot_seed(keystore, community_id, &seed_hex);
        crate::keystore::persist_slot_keypair(keystore, community_id, &slot_kp_str);
    }

    // Persist subkey_index to SQLite
    {
        let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
        let cid = community_id.to_string();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        let idx = i64::from(slot_index);
        crate::db_helpers::db_fire(pool.inner(), "persist my_subkey_index from seed", move |conn| {
            conn.execute(
                "UPDATE communities SET my_subkey_index = ?1 WHERE owner_key = ?2 AND id = ?3",
                rusqlite::params![idx, owner_key, cid],
            )?;
            Ok(())
        });
    }

    tracing::info!(
        community = %community_id,
        slot_index,
        "slot seed received — derived slot keypair locally (no coordinator needed)"
    );

    // Trigger immediate presence write
    let state_for_poll = state.clone();
    let cid_for_poll = community_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = crate::services::community::presence_poll_tick_public(
            &state_for_poll,
            &cid_for_poll,
        )
        .await
        {
            tracing::debug!(error = %e, "immediate presence poll after slot seed grant failed");
        }
    });
}

/// Handle a sync request — respond with messages from local SQLite.
/// Check if a gossip moderation sender has the required permission.
///
/// Uses the `member_roles` cache in CommunityState (populated by presence_poll_tick).
/// Returns `true` if the sender has the permission, `false` if denied.
fn check_gossip_moderation_permission(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: &rekindle_protocol::dht::community::envelope::ControlPayload,
) -> bool {
    use rekindle_protocol::dht::community::envelope::ControlPayload;
    use rekindle_protocol::dht::community::permissions_v2::Permissions;

    let required = match payload {
        ControlPayload::Kick { .. } => Permissions::KICK_MEMBERS,
        ControlPayload::Ban { .. } | ControlPayload::Unban { .. } => Permissions::BAN_MEMBERS,
        ControlPayload::TimeoutMember { .. } | ControlPayload::RemoveTimeout { .. } => Permissions::MODERATE_MEMBERS,
        ControlPayload::AssignRole { .. } | ControlPayload::UnassignRole { .. } => Permissions::MANAGE_ROLES,
        ControlPayload::EditRole { .. } => Permissions::ADMINISTRATOR,
        _ => return true, // Non-moderation payloads pass through
    };

    let communities = state.communities.read();
    let Some(cs) = communities.get(community_id) else { return false };

    let sender_role_ids = cs.member_roles.get(sender_pseudonym).cloned();
    match sender_role_ids {
        Some(ref role_ids) => {
            let roles_v2: Vec<rekindle_protocol::dht::community::types::RoleEntryV2> =
                cs.roles.iter().map(|r| rekindle_protocol::dht::community::types::RoleEntryV2 {
                    id: r.id,
                    name: r.name.clone(),
                    color: r.color,
                    permissions: r.permissions,
                    position: r.position,
                    hoist: r.hoist,
                    mentionable: r.mentionable,
                }).collect();
            drop(communities); // release lock before calling is_owner_by_roles (which also reads)
            let is_owner = crate::services::coordinator::state_manager::is_owner_by_roles(
                state, community_id, sender_pseudonym, role_ids,
            );
            let perms = rekindle_protocol::dht::community::permissions_v2::calculate_permissions_v2(
                role_ids, &roles_v2, &[], sender_pseudonym, is_owner, None,
            );
            if perms.has(required) {
                true
            } else {
                tracing::warn!(
                    community = %community_id,
                    sender = %sender_pseudonym,
                    required = ?required,
                    "gossip moderation: sender lacks required permission — ignoring"
                );
                false
            }
        }
        None => {
            // Sender not in member_roles cache — accept if they're a known member.
            // Their roles will be cached on next presence poll.
            cs.known_members.contains(sender_pseudonym)
        }
    }
}

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

/// Bundled JoinAccepted data to avoid too-many-arguments clippy lint.
struct JoinAcceptedData<'a> {
    mek_wire_bytes: &'a [u8],
    mek_generation: u64,
    channels: &'a [serde_json::Value],
    role_ids: &'a [u32],
    roles: &'a [serde_json::Value],
    members: &'a [serde_json::Value],
    member_registry_key: Option<&'a str>,
    slot_index: Option<u32>,
    wrapped_slot_seed: Option<&'a [u8]>,
    wrapped_slot_keypair: Option<&'a [u8]>,
}

/// Handle a JoinAccepted response from an admin peer.
///
/// Updates local community state with fresh MEK, roles, channels, members,
/// and member_registry_key. Persists to SQLite so state survives restarts.
async fn handle_join_accepted(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    data: &JoinAcceptedData<'_>,
) {
    use crate::channels::CommunityEvent;
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_protocol::dht::community::types::MemberSummary;

    // Guard: ignore self-JoinAccepted (loopback). Admins who process joins
    // already have correct state — processing our own JoinAccepted would
    // overwrite role_ids with potentially stale data.
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
    };
    if my_pseudonym.as_deref() == Some(sender_pseudonym) {
        tracing::debug!(
            community = %community_id,
            "ignoring self-JoinAccepted — we are the coordinator"
        );
        return;
    }

    // 1. Restore and cache the MEK
    if !data.mek_wire_bytes.is_empty() {
        if let Some(mek) = MediaEncryptionKey::from_wire_bytes(data.mek_wire_bytes) {
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
    let parsed_members: Vec<MemberSummary> = data.members
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();

    // 2a. Extract our own subkey_index from members list as backup.
    // Primary path: data.slot_index → handle_slot_seed_grant() (line ~2786).
    // Backup path: if data.slot_index is None, recover from members list.
    if let Some(ref my_pk) = my_pseudonym {
        if let Some(me) = parsed_members.iter().find(|m| m.pseudonym_key == *my_pk) {
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id) {
                if cs.my_subkey_index.is_none() {
                    cs.my_subkey_index = Some(me.subkey_index);
                    tracing::info!(
                        community = %community_id,
                        subkey_index = me.subkey_index,
                        "extracted my_subkey_index from members list (backup path)"
                    );
                }
                // Also update role_ids from authoritative member registry data
                if !me.role_ids.is_empty() && me.role_ids.len() >= cs.my_role_ids.len() {
                    cs.my_role_ids.clone_from(&me.role_ids);
                }
            }
        }
    }

    // Persist the backup subkey_index to SQLite when primary path (slot_index) is absent
    if data.slot_index.is_none() {
        if let Some(ref my_pk) = my_pseudonym {
            if let Some(me) = parsed_members.iter().find(|m| m.pseudonym_key == *my_pk) {
                let pool: tauri::State<'_, DbPool> = app_handle.state();
                let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
                let cid = community_id.to_string();
                let idx = i64::from(me.subkey_index);
                crate::db_helpers::db_fire(
                    pool.inner(),
                    "backup my_subkey_index from members list",
                    move |conn| {
                        conn.execute(
                            "UPDATE communities SET my_subkey_index = ?1 WHERE owner_key = ?2 AND id = ?3",
                            rusqlite::params![idx, owner_key, cid],
                        )?;
                        Ok(())
                    },
                );
            }
        }
    }

    // 3. Parse roles from coordinator
    let parsed_roles: Vec<crate::state::RoleDefinition> = data.roles
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
            cs.mek_generation = data.mek_generation;
            // Monotonic role_ids protection: never overwrite with a shorter list.
            // A stale or corrupted JoinAccepted could carry [0,1] instead of
            // the full [0,1,2,3,4] — accepting that would strip permissions.
            if !data.role_ids.is_empty() && data.role_ids.len() >= cs.my_role_ids.len() {
                cs.my_role_ids = data.role_ids.to_vec();
            } else if !data.role_ids.is_empty() {
                tracing::warn!(
                    community = %community_id,
                    incoming = data.role_ids.len(),
                    current = cs.my_role_ids.len(),
                    "rejecting role_ids update — incoming list shorter (monotonic guard)"
                );
            }
            if !parsed_roles.is_empty() {
                cs.roles.clone_from(&parsed_roles);
            }

            // Update channel list if provided, and extract log_keys
            if !data.channels.is_empty() {
                let mut updated_channels = Vec::new();
                for ch_val in data.channels {
                    if let Ok(ch) =
                        serde_json::from_value::<crate::state::ChannelInfo>(ch_val.clone())
                    {
                        // Extract log_key from the raw JSON (ChannelEntryV2 has it, ChannelInfo doesn't)
                        if let Some(log_key) = ch_val.get("log_key").and_then(|v| v.as_str()) {
                            cs.channel_log_keys.insert(ch.id.clone(), log_key.to_string());
                        }
                        updated_channels.push(ch);
                    }
                }
                if !updated_channels.is_empty() {
                    cs.channels = updated_channels;
                }
            }

            // Set member_registry_key from coordinator
            if let Some(rk) = data.member_registry_key {
                cs.member_registry_key = Some(rk.to_string());
            }
        }
    }

    // Channel log keypairs no longer distributed — SMPL records use slot seed.

    // 4b. Persist member_registry_key, my_role_ids, and roles to SQLite so
    //     state survives restarts. Without this, my_role_ids and roles loaded
    //     on next login would be stale (from the original join/create time).
    {
        let pool: tauri::State<'_, DbPool> = app_handle.state();
        let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
        let cid = community_id.to_string();
        let rk_str = data.member_registry_key.map(str::to_string);
        let role_ids_json = serde_json::to_string(data.role_ids).unwrap_or_else(|_| "[]".into());
        let roles_for_db = parsed_roles.clone();
        let _ = crate::db_helpers::db_call(pool.inner(), move |conn| {
            // Update my_role_ids and optionally member_registry_key
            if let Some(ref rk) = rk_str {
                conn.execute(
                    "UPDATE communities SET member_registry_key = ?1, my_role_ids = ?2 \
                     WHERE owner_key = ?3 AND id = ?4",
                    rusqlite::params![rk, role_ids_json, owner_key, cid],
                )?;
            } else {
                conn.execute(
                    "UPDATE communities SET my_role_ids = ?1 \
                     WHERE owner_key = ?2 AND id = ?3",
                    rusqlite::params![role_ids_json, owner_key, cid],
                )?;
            }
            // Persist role definitions so they survive restarts
            if !roles_for_db.is_empty() {
                conn.execute(
                    "DELETE FROM community_roles WHERE owner_key = ?1 AND community_id = ?2",
                    rusqlite::params![owner_key, cid],
                )?;
                for r in &roles_for_db {
                    conn.execute(
                        "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        rusqlite::params![
                            owner_key, cid, r.id, r.name, r.color,
                            i64::try_from(r.permissions).unwrap_or(0), r.position,
                            i32::from(r.hoist), i32::from(r.mentionable),
                        ],
                    )?;
                }
            }
            Ok(())
        })
        .await;
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

    // 5b. Populate known_members cache from the received member list
    if !parsed_members.is_empty() {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            for m in &parsed_members {
                cs.known_members.insert(m.pseudonym_key.clone());
            }
        }
    }

    // 6. Persist MEK to Stronghold so it survives app restarts.
    //    The persist_mek call in join_community_command runs BEFORE JoinAccepted
    //    arrives (join is async fire-and-forget), so it finds nothing in mek_cache.
    //    We must persist here, after the MEK is cached.
    if !data.mek_wire_bytes.is_empty() {
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

    // 6b. Process bundled slot_seed (preferred) or legacy slot_keypair.
    // With slot_seed, the member derives their own keypair locally — no coordinator needed.
    if let (Some(idx), Some(wrapped_seed)) = (data.slot_index, data.wrapped_slot_seed) {
        handle_slot_seed_grant(
            app_handle,
            state,
            community_id,
            sender_pseudonym,
            idx,
            wrapped_seed,
        );
    } else if let (Some(idx), Some(wrapped)) = (data.slot_index, data.wrapped_slot_keypair) {
        // Legacy fallback: older coordinator sent wrapped_slot_keypair directly
        handle_slot_keypair_grant(
            app_handle,
            state,
            community_id,
            sender_pseudonym,
            idx,
            0, // segment_index
            wrapped,
        );
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
        mek_generation = data.mek_generation,
        role_ids = ?data.role_ids,
        member_count = parsed_members.len(),
        has_slot_keypair = data.slot_index.is_some(),
        "JoinAccepted processed — community state updated"
    );

    // Bootstrap gossip peers from JoinAccepted member list
    spawn_peer_bootstrap(state.clone(), community_id.to_string(), parsed_members);
}

/// Bootstrap gossip.peers by fetching MemberPresence (route_blobs) from DHT.
///
/// MemberSummary (from member index) doesn't have route_blob — must read each member's
/// SMPL presence subkey. This gives the joiner instant connectivity without waiting
/// for the 60-second presence poll to discover peers.
fn spawn_peer_bootstrap(
    state: Arc<AppState>,
    community_id: String,
    members: Vec<rekindle_protocol::dht::community::types::MemberSummary>,
) {
    tokio::spawn(async move {
        let (registry_key, my_pseudo) = {
            let communities = state.communities.read();
            let cs = communities.get(&community_id);
            (
                cs.and_then(|c| c.member_registry_key.clone()),
                cs.and_then(|c| c.my_pseudonym_key.clone()),
            )
        };
        let Some(rk) = registry_key else { return };
        let Some(rc) = crate::state_helpers::routing_context(&state) else { return };
        let mgr = rekindle_protocol::dht::DHTManager::new(rc);

        if let Err(e) = mgr.open_record(&rk).await {
            tracing::debug!(error = %e, "failed to open registry for peer bootstrap");
            return;
        }

        // Stagger DHT reads to avoid overwhelming Veilid connections during bootstrap
        let mut found_peers = 0u32;
        for member in &members {
            if my_pseudo.as_deref() == Some(&member.pseudonym_key) {
                continue;
            }
            // Small delay between reads to avoid connection burst
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if let Ok(Some(presence)) =
                rekindle_protocol::dht::community::member_registry::read_member_presence(
                    &mgr, &rk, member.subkey_index,
                )
                .await
            {
                if let Some(blob) = presence.route_blob {
                    if !blob.is_empty() && presence.status != "offline" {
                        let mut communities = state.communities.write();
                        if let Some(cs) = communities.get_mut(&community_id) {
                            if let Some(ref mut gossip) = cs.gossip {
                                let om = crate::state::OnlineMember {
                                    route_blob: blob,
                                    last_seen: rekindle_utils::timestamp_secs(),
                                };
                                gossip.online_members.insert(
                                    member.pseudonym_key.clone(),
                                    om.clone(),
                                );
                                gossip.peers.insert(member.pseudonym_key.clone(), om);
                                found_peers += 1;
                            }
                        }
                    }
                }
            }
        }
        if found_peers > 0 {
            tracing::info!(
                community = %community_id,
                peers = found_peers,
                "bootstrapped gossip peers from JoinAccepted member list"
            );
        }
    });
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
    sequence: u64,
}

/// Handle a `NewMessage` community broadcast: decrypt, store, and emit.
async fn handle_broadcast_new_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    msg: &BroadcastNewMessage,
) {
    tracing::info!(
        community = %msg.community_id,
        channel = %msg.channel_id,
        sender = %msg.sender_pseudonym,
        "handle_broadcast_new_message: received channel message"
    );

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
        MekDecryptResult::Failed => {
            tracing::warn!(
                community = %msg.community_id,
                channel = %msg.channel_id,
                sender = %msg.sender_pseudonym,
                mek_gen = msg.mek_generation,
                "MEK decrypt failed — message dropped (no cached MEK or decrypt error)"
            );
            return;
        }
        MekDecryptResult::NeedRefresh => {
            fetch_mek_from_dht(app_handle, state, &msg.community_id).await;

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
                tracing::warn!(
                    community = %msg.community_id,
                    channel = %msg.channel_id,
                    sender = %msg.sender_pseudonym,
                    mek_gen = msg.mek_generation,
                    "MEK still mismatched after DHT refresh — message dropped"
                );
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

    // Sequence gap detection (Briar-inspired) — detect missing messages from this sender
    if msg.sequence > 0 {
        let key = (msg.sender_pseudonym.clone(), msg.channel_id.clone());
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&msg.community_id) {
            let last_seen = cs.peer_sequences.get(&key).copied().unwrap_or(0);
            if msg.sequence > last_seen + 1 && last_seen > 0 {
                tracing::warn!(
                    sender = %msg.sender_pseudonym,
                    channel = %msg.channel_id,
                    expected = last_seen + 1,
                    received = msg.sequence,
                    gap = msg.sequence - last_seen - 1,
                    "sequence gap detected — {} messages may be missing",
                    msg.sequence - last_seen - 1
                );
                // Queue a SyncRequest for the gap
                cs.pending_syncs.entry(msg.channel_id.clone())
                    .or_insert((rekindle_utils::timestamp_secs(), 0));
            }
            cs.peer_sequences.insert(key, msg.sequence);
        }
    }

    // Resolve sender's display name from SQLite (community_members table).
    // This prevents the frontend from showing raw pseudonym public keys.
    let sender_display_name = {
        let pool: tauri::State<'_, DbPool> = app_handle.state();
        let owner = state_helpers::owner_key_or_default(state);
        let cid = msg.community_id.clone();
        let spn = msg.sender_pseudonym.clone();
        crate::db_helpers::db_call(pool.inner(), move |conn| {
            Ok(conn.query_row(
                "SELECT display_name FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                rusqlite::params![owner, cid, spn],
                |row| row.get::<_, String>(0),
            ).ok())
        }).await.unwrap_or_default()
    };

    // Emit to frontend
    let event = crate::channels::ChatEvent::MessageReceived {
        from: msg.sender_pseudonym.clone(),
        body,
        timestamp: msg.timestamp,
        conversation_id: msg.channel_id.clone(),
        server_message_id: Some(msg.message_id.clone()),
        reply_to_id: msg.reply_to_id.clone(),
        sender_display_name,
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
                "MEK generation mismatch — fetching updated MEK from DHT vault"
            );
            MekDecryptResult::NeedRefresh
        }
        None => {
            tracing::warn!(community = %community_id, "no MEK cached for community");
            MekDecryptResult::Failed
        }
    }
}


/// Fetch the current MEK from the DHT MEK vault (registry subkey 1).
///
/// Reads the per-member encrypted MEK vault, finds our own entry, unwraps it
/// using ECDH, and updates `mek_cache` + Stronghold.
pub(super) async fn fetch_mek_from_dht(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
) {
    use rekindle_crypto::group::mek_distribution::unwrap_mek;
    use rekindle_protocol::dht::community::member_registry;

    let (registry_key, my_pseudonym, my_signing_key) = {
        let communities = state.communities.read();
        let Some(c) = communities.get(community_id) else {
            tracing::warn!(community = %community_id, "fetch_mek_from_dht: community not found");
            return;
        };
        let Some(registry_key) = c.member_registry_key.clone() else {
            tracing::warn!(community = %community_id, "fetch_mek_from_dht: no registry key");
            return;
        };
        let my_pseudonym = c.my_pseudonym_key.clone().unwrap_or_default();
        let secret = state.identity_secret.lock();
        let Some(ref s) = *secret else {
            tracing::warn!(community = %community_id, "fetch_mek_from_dht: no identity secret");
            return;
        };
        let signing_key =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(s, community_id);
        (registry_key, my_pseudonym, signing_key)
    };

    let Some(rc) = crate::state_helpers::routing_context(state) else {
        tracing::warn!(community = %community_id, "fetch_mek_from_dht: not attached");
        return;
    };
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    // Read MEK vault from registry subkey 1
    let vault = match member_registry::read_mek_vault(&mgr, &registry_key).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, community = %community_id, "failed to read MEK vault from DHT");
            return;
        }
    };

    // Find community-wide MEK entry (channel_id is empty)
    let Some(entry) = vault.iter().find(|e| e.channel_id.is_empty()) else {
        tracing::debug!(community = %community_id, "no community-wide MEK entry in vault");
        return;
    };

    // Find our own encrypted copy
    let Some(copy) = entry.copies.iter().find(|c| c.target_pseudonym == my_pseudonym) else {
        tracing::warn!(community = %community_id, "no MEK copy for us in vault");
        return;
    };

    // Get rotator's public key for ECDH unwrapping
    let Some(rotator_pub): Option<[u8; 32]> = hex::decode(&entry.rotator_pseudonym)
        .ok()
        .and_then(|b| b.try_into().ok())
    else {
        tracing::warn!(community = %community_id, "invalid rotator pseudonym key");
        return;
    };

    // Unwrap MEK
    let mek_wire = match unwrap_mek(&my_signing_key, &rotator_pub, &copy.encrypted_mek) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, community = %community_id, "failed to unwrap MEK from vault");
            return;
        }
    };

    let Some(mek) = rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&mek_wire)
    else {
        tracing::warn!(community = %community_id, "invalid MEK wire bytes from vault");
        return;
    };

    let new_gen = mek.generation();
    tracing::info!(community = %community_id, generation = new_gen, "MEK fetched from DHT vault");

    // Update local state
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.mek_generation = new_gen;
        }
    }
    state
        .mek_cache
        .lock()
        .insert(community_id.to_string(), mek);

    // Persist to Stronghold
    let keystore_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let ks_guard = keystore_handle.lock();
    if let Some(ref ks) = *ks_guard {
        if let Some(mek) = state.mek_cache.lock().get(community_id) {
            crate::keystore::persist_mek(ks, community_id, mek);
        }
    }
    drop(ks_guard);
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
        let mut cleared_coordinator_routes: Vec<String> = Vec::new();
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
                            cleared_coordinator_routes.push(community.id.clone());
                        }
                    }
                }
            }
        }

        // Re-fetch coordinator route from DHT for communities whose routes died
        for cid in cleared_coordinator_routes {
            let state = state.clone();
            tokio::spawn(async move {
                let manifest_key = {
                    let communities = state.communities.read();
                    communities.get(&cid).and_then(|cs| cs.manifest_key.clone())
                };
                if let (Some(mk), Some(rc)) = (manifest_key, crate::state_helpers::routing_context(&state)) {
                    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
                    match rekindle_protocol::dht::community::manifest::read_coordinator(&mgr, &mk).await {
                        Ok(Some(coord_info)) => {
                            let mut communities = state.communities.write();
                            if let Some(cs) = communities.get_mut(&cid) {
                                cs.coordinator_route_blob = Some(coord_info.route_blob);
                                cs.coordinator_pseudonym = Some(coord_info.pseudonym_key);
                                tracing::info!(community = %cid, "recovered coordinator route from DHT manifest");
                            }
                        }
                        Ok(None) => tracing::debug!(community = %cid, "no coordinator info in manifest"),
                        Err(e) => tracing::debug!(community = %cid, error = %e, "failed to re-fetch coordinator from DHT"),
                    }
                }
            });
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
                    // For other communities, re-announce presence via DHT registry.
                    let all_community_ids: Vec<String> = {
                        let communities = state.communities.read();
                        communities.keys().cloned().collect()
                    };
                    // Static owner model — no heartbeat writes needed on route refresh.
                    // The presence poll below handles publishing our new route_blob
                    // to the SMPL registry, which is how peers discover our route.

                    // Reset needs_initial_sync so the next presence poll re-syncs
                    {
                        let mut communities = state.communities.write();
                        for community_id in &all_community_ids {
                            if let Some(cs) = communities.get_mut(community_id) {
                                if let Some(ref mut gossip) = cs.gossip {
                                    gossip.needs_initial_sync = true;
                                }
                            }
                        }
                    }

                    // Trigger presence poll for all communities to publish new route
                    for community_id in &all_community_ids {
                        let _ = crate::services::community::rejoin_community(
                            &state,
                            community_id,
                        )
                        .await;
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

    // 7. Clear community-specific state (including all crypto caches)
    state.mek_cache.lock().clear();
    state.channel_mek_cache.lock().clear();
    state.dedup_cache.lock().clear();

    // 8. Stop coordinator services
    {
        let services = std::mem::take(&mut *state.coordinator_services.write());
        drop(services);
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
