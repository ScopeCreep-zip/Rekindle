//! Service dispatcher: thin router from IPC requests to ChatService methods.
//!
//! Every `IpcRequest` variant maps to exactly one `ChatService` method call.
//! No business logic. No crypto. No persistence. Single-expression match arms.
//!
//! Lifecycle operations (Status, Unlock, Lock, Shutdown) are handled directly
//! because they manage the daemon state machine, vault open/close, and transport
//! start/stop — concerns that belong to the daemon, not the chat application.
//!
//! Lock discipline: `parking_lot::RwLock` guards are NEVER held across `.await`.
//! Pattern: clone `Arc` from `RwLock`, drop guard, then `.await` on the clone.

pub(crate) mod admin;
pub(crate) mod bulk_transfers;
pub(crate) mod lifecycle;

use std::sync::Arc;
use std::time::Instant;

use rekindle_types::display::StatusSnapshot;
use rekindle_types::transport::Transport;

use crate::daemon::DaemonState;
use crate::ipc::protocol::{IpcRequest, IpcResponse};
use crate::ipc::registry::ClearanceRegistry;
use crate::state::StatePaths;

/// Active authorization policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    pub min_hop_count: Option<u8>,
    #[serde(default)]
    pub require_signature_verification: bool,
    pub max_gossip_ttl: Option<u8>,
}

/// Daemon context — holds ChatService + daemon-specific concerns.
///
/// Business logic state (sessions, MEK cache, signing key, watches) lives
/// inside `ChatService`. The daemon owns lifecycle, IPC registry, policy,
/// and event delivery plumbing.
///
/// Mutable fields use `parking_lot::RwLock` for interior mutability —
/// `DaemonContext` is shared via `Arc` across IPC handler tasks.
/// `handle_unlock` writes to `chat`, `transport`, and `vault`.
/// All other handlers read-only (clone Arc, drop guard, await).
pub struct DaemonContext {
    /// The chat application service. `None` before unlock. Set during
    /// UNLOCKING → RESUMING when the vault opens and transport attaches.
    /// `RwLock` for interior mutability — `handle_unlock` sets, dispatch reads.
    pub chat: parking_lot::RwLock<Option<Arc<rekindle_chat::ChatService>>>,

    /// The transport (`Arc<dyn Transport>`). Set during unlock.
    /// Used by lifecycle handlers for shutdown. `None` before unlock.
    pub transport: parking_lot::RwLock<Option<Arc<dyn Transport>>>,

    /// The vault store. Set during unlock. Used by lifecycle for close.
    /// `None` before unlock. VaultStore::Drop does PRAGMA rekey cleanup.
    pub vault: parking_lot::RwLock<Option<Arc<rekindle_storage::VaultStore>>>,

    /// Current daemon lifecycle state.
    pub lifecycle: Arc<crate::daemon::DaemonLifecycle>,

    /// Resolved XDG paths (state_dir, session_file, vault_db, config_dir, veilid_dir).
    pub paths: StatePaths,

    /// Agent identity and clearance registry (IPC concern).
    pub registry: Arc<tokio::sync::RwLock<ClearanceRegistry>>,

    /// Active authorization policy.
    pub policy: parking_lot::RwLock<PolicyConfig>,

    /// Watch channel sender for IPC event delivery plumbing.
    pub event_watch_tx: tokio::sync::watch::Sender<
        Option<tokio::sync::broadcast::Sender<rekindle_types::subscription_events::SubscriptionEvent>>,
    >,

    /// Cached status snapshot with TTL. Refreshed at most once per second.
    /// Prevents lock contention on ChatService internals when many agents
    /// poll status simultaneously (monitoring fleet).
    pub status_cache: parking_lot::Mutex<Option<(Instant, StatusSnapshot)>>,

    /// Dedicated rayon thread pool for CPU-bound bulk encryption/decryption.
    pub encrypt_pool: Arc<rayon::ThreadPool>,

    /// Shared atomic counters for bulk transfer observability.
    /// One Arc cloned into both ServerState and DaemonContext.
    pub bulk_counters: Arc<crate::ipc::bulk::BulkCounters>,

    /// Registry of active and recent bulk transfers for observability.
    pub bulk_transfers: parking_lot::Mutex<bulk_transfers::BulkTransferRegistry>,

    /// Shared event journal for cursor-based resumption.
    /// Events are appended here by the event delivery task.
    /// Clients replay from here on reconnect via EventResume.
    pub event_journal: Arc<crate::ipc::journal::EventJournal>,

    /// Idempotency cache for exactly-once request processing.
    /// Checked before dispatching mutating requests that carry
    /// a client_msg_id.
    pub idempotency_cache: crate::ipc::idempotency::IdempotencyCache,

    /// Cached crypto capability probe results. Computed once at daemon
    /// startup. Read by `build_checks()` for `status --doctor` without
    /// re-running the ~200ms benchmark on every request.
    pub crypto_caps: crate::ipc::bulk::capability::CryptoCapabilities,

    /// Reference to the IPC bus server's shared state.
    /// Used by bulk transfer cancel to signal the connection's reassembler.
    pub server_state: parking_lot::RwLock<Option<Arc<crate::ipc::server::state::ServerState>>>,
}

/// Dispatch an IPC request to the appropriate handler.
pub async fn dispatch(ctx: &Arc<DaemonContext>, request: IpcRequest, sender_name: Option<Arc<str>>) -> IpcResponse {
    let state = ctx.lifecycle.state();

    match request {
        // ── Lifecycle (any state) ────────────────────────────────
        IpcRequest::Status => lifecycle::handle_status(ctx, state),
        IpcRequest::NetworkStatus => admin::handle_network_status(ctx, state),
        IpcRequest::NetworkPeers => admin::handle_network_peers(ctx, state),
        IpcRequest::Unlock { passphrase } => {
            lifecycle::handle_unlock(ctx, state, &passphrase).await
        }
        IpcRequest::Lock => lifecycle::handle_lock(ctx, state).await,
        IpcRequest::Shutdown => lifecycle::handle_shutdown(ctx, state).await,
        IpcRequest::AgentRegister { name, agent_type, capabilities } =>
            admin::handle_agent_register(ctx, &name, agent_type, &capabilities),
        IpcRequest::AgentRevoke { name } =>
            admin::handle_agent_revoke(ctx, &name),
        IpcRequest::PolicyReload => admin::handle_policy_reload(ctx),

        // ── Bulk Transfer (control-plane signaling) ──────────────
        // Actual bulk data flows through the bulk lane (0x01–0x02),
        // not through IpcRequest. These control messages coordinate
        // transfer lifecycle (start, complete, cancel, status).
        IpcRequest::BulkTransferStart { transfer_id, total_size, media_type, digest, direction } => {
            let conn_id = sender_name.as_deref().and_then(|name| {
                ctx.server_state.read().as_ref().and_then(|ss| {
                    ss.name_to_conn.read().get(name).copied()
                })
            }).unwrap_or(0);

            let nonce_counter = ctx.server_state.read().as_ref().and_then(|ss| {
                ss.connections.get(&conn_id).and_then(|conn| {
                    conn.bulk_nonce_counter.clone()
                })
            });

            let stream_id = ctx.bulk_transfers.lock().start(
                transfer_id.clone(), total_size, media_type, digest, direction.clone(),
                conn_id, nonce_counter,
                crate::ipc::bulk::verify::DigestAlgorithm::default(),
            );
            tracing::info!(transfer_id, total_size, stream_id, conn_id, direction, "bulk transfer started");
            IpcResponse::ok(&serde_json::json!({
                "transfer_id": transfer_id,
                "stream_id": stream_id,
                "status": "active",
            }))
        }
        IpcRequest::BulkTransferComplete { transfer_id, digest, bytes_transferred } => {
            // Read the completed payload from the connection's delivery channel.
            let delivered_payload = {
                let conn_id = ctx.bulk_transfers.lock().status(&transfer_id)
                    .map_or(0, |s| s.conn_id);
                ctx.server_state.read().as_ref().and_then(|ss| {
                    ss.connections.get(&conn_id).and_then(|conn| {
                        conn.bulk_deliver_rx.lock().try_recv().ok()
                    })
                })
            };

            let found = ctx.bulk_transfers.lock().complete(&transfer_id, bytes_transferred);
            if found {
                let payload_size = delivered_payload.as_ref().map_or(0, |(_, p)| p.len());
                tracing::info!(
                    transfer_id, digest, bytes_transferred, payload_size,
                    "bulk transfer completed"
                );
                IpcResponse::ok(&serde_json::json!({
                    "transfer_id": transfer_id,
                    "status": "completed",
                    "payload_size": payload_size,
                }))
            } else {
                IpcResponse::error(404, format!("transfer {transfer_id} not found"))
            }
        }
        IpcRequest::BulkTransferCancel { transfer_id, reason } => {
            let cancel_info = {
                let mut reg = ctx.bulk_transfers.lock();
                let info = reg.status(&transfer_id).map(|s| {
                    let nonce = s.nonce_counter.as_ref()
                        .map_or(0, |n| n.current());
                    (s.conn_id, nonce)
                });
                reg.cancel(&transfer_id);
                info
            };

            if let Some((conn_id, next_nonce)) = cancel_info {
                if let Some(ss) = ctx.server_state.read().as_ref() {
                    ss.cancel_bulk_stream(conn_id, next_nonce);
                }
                tracing::info!(transfer_id, reason, conn_id, next_nonce, "bulk transfer cancelled");
                IpcResponse::ok(&serde_json::json!({
                    "transfer_id": transfer_id,
                    "status": "cancelled",
                }))
            } else {
                IpcResponse::error(404, format!("transfer {transfer_id} not found"))
            }
        }
        IpcRequest::BulkTransferStatus { transfer_id } => {
            match ctx.bulk_transfers.lock().status(&transfer_id) {
                Some(state) => IpcResponse::ok(&state),
                None => IpcResponse::error(404, format!("transfer {transfer_id} not found")),
            }
        }

        // ── Event Journal Resume ─────────────────────────────────
        IpcRequest::EventResume { last_seen_seq } => {
            match ctx.event_journal.replay_from(last_seen_seq) {
                Ok(events) => {
                    let event_data: Vec<serde_json::Value> = events
                        .iter()
                        .map(|e| serde_json::json!({
                            "seq": e.seq,
                            "event": *e.event,
                        }))
                        .collect();
                    IpcResponse::ok(&serde_json::json!({
                        "replayed": events.len(),
                        "current_head": ctx.event_journal.head_seq(),
                        "events": event_data,
                    }))
                }
                Err(e) => IpcResponse::error_with_remediation(
                    410,
                    format!("{e}"),
                    "re-fetch full state with Status + CommunityList + FriendList",
                ),
            }
        }

        // ── Everything else requires OPERATIONAL state + ChatService ──
        _ => {
            if !state.can_query() {
                return state_error(state, "query");
            }
            let chat = ctx.chat.read().clone();
            let Some(chat) = chat else {
                return IpcResponse::error(503, "chat service not initialized — unlock first");
            };

            // Extract client_msg_id before moving request into dispatch.
            // This avoids cloning the entire IpcRequest on every dispatch.
            let client_msg_id = match &request {
                IpcRequest::ChannelSend { client_msg_id: Some(id), .. } => {
                    if let Some(cached) = ctx.idempotency_cache.check(id) {
                        return cached;
                    }
                    Some(id.clone())
                }
                _ => None,
            };

            let response = dispatch_to_chat(&chat, request, state).await;

            // Cache the response for idempotent requests.
            if let Some(id) = client_msg_id {
                ctx.idempotency_cache.store(id, response.clone());
            }

            response
        }
    }
}

/// Forward business logic requests to ChatService methods.
/// Every match arm is a single expression.
async fn dispatch_to_chat(
    chat: &rekindle_chat::ChatService,
    request: IpcRequest,
    _state: DaemonState,
) -> IpcResponse {
    match request {
        // ── Identity ─────────────────────────────────────────────
        IpcRequest::IdentityCreate { display_name } =>
            map_result(chat.init_identity(&display_name).await),
        IpcRequest::IdentityDestroy { .. } =>
            map_result(chat.destroy_identity().await),
        IpcRequest::IdentityExportEncrypted { passphrase } =>
            map_result(chat.identity_export_encrypted(&passphrase)),
        IpcRequest::IdentityImportEncrypted { passphrase, data } =>
            map_result(chat.identity_import_encrypted(&passphrase, &data)),
        IpcRequest::IdentityImport { data } =>
            map_result(chat.identity_import(&data)),

        // ── Community ────────────────────────────────────────────
        IpcRequest::CommunityCreate { name, description } =>
            map_result(chat.create_community(&name, &description).await),
        IpcRequest::CommunityJoin { invite } =>
            map_result(chat.join_community(&invite).await),
        IpcRequest::CommunityLeave { governance_key } =>
            map_result(chat.leave_community(&governance_key).await),

        // ── Channel ──────────────────────────────────────────────
        IpcRequest::ChannelSend { community, channel, body, reply_to, .. } =>
            map_result(chat.send_channel_message(&community, &channel, &body, reply_to).await),
        IpcRequest::ChannelTyping { community, channel } =>
            map_result(chat.send_channel_typing(&community, &channel).await),

        // ── Social (friends + DMs) ───────────────────────────────
        IpcRequest::FriendAdd { target_profile_key, message } =>
            map_result(chat.send_friend_request(&target_profile_key, &message).await),
        IpcRequest::FriendAccept { public_key } =>
            map_result(chat.accept_friend_request(&public_key).await),
        IpcRequest::FriendReject { public_key } =>
            map_result(chat.reject_friend_request(&public_key).await),
        IpcRequest::FriendRemove { public_key } =>
            map_result(chat.remove_friend(&public_key).await),
        IpcRequest::FriendRequests =>
            IpcResponse::ok(&chat.list_pending_requests()),
        IpcRequest::DmSend { peer_key, body } =>
            map_result(chat.send_dm(&peer_key, &body).await),
        IpcRequest::DmThread { peer_key, limit } =>
            map_result(chat.dm_thread(&peer_key, limit)),

        // ── Presence ─────────────────────────────────────────────
        IpcRequest::PresenceSet { status, message } =>
            map_result(chat.set_presence(&status, message.as_deref()).await),
        IpcRequest::GamePresenceClear =>
            map_result(chat.set_presence("online", None).await),

        // ── Social ────────────────────────────────────────────────
        IpcRequest::ReactionAdd { community, channel, message_id, emoji } =>
            map_result(chat.add_reaction(&community, &channel, &message_id, &emoji).await),
        IpcRequest::ReactionRemove { community, channel, message_id, emoji } =>
            map_result(chat.remove_reaction(&community, &channel, &message_id, &emoji).await),
        IpcRequest::PinAdd { community, channel, message_id } =>
            map_result(chat.pin_message(&community, &channel, &message_id).await),
        IpcRequest::PinRemove { community, channel, message_id } =>
            map_result(chat.unpin_message(&community, &channel, &message_id).await),
        IpcRequest::EventCreate { community, title, description, start_time, end_time, channel_id, max_attendees } =>
            map_result(chat.create_event(&community, &title, &description, start_time, end_time, channel_id.as_deref(), max_attendees).await),
        IpcRequest::EventUpdate { community, event_id, title, description, start_time, end_time, max_attendees } =>
            map_result(chat.update_event(&community, &event_id, &title, &description, start_time, end_time, max_attendees).await),
        IpcRequest::EventDelete { community, event_id } =>
            map_result(chat.delete_event(&community, &event_id).await),
        IpcRequest::EventRsvp { community, event_id, status } =>
            map_result(chat.rsvp_event(&community, &event_id, &status).await),
        IpcRequest::EventRemind { community, event_id, title, minutes_until } =>
            map_result(chat.event_reminder(&community, &event_id, &title, minutes_until).await),
        IpcRequest::ThreadCreate { community, channel, parent_message_id, title, auto_archive_seconds } =>
            map_result(chat.create_thread(&community, &channel, &parent_message_id, &title, auto_archive_seconds).await),
        IpcRequest::ThreadMessage { community, thread_id, ciphertext, mek_generation, reply_to_id } =>
            map_result(chat.thread_message(&community, &thread_id, ciphertext, mek_generation, reply_to_id.as_deref()).await),
        IpcRequest::ThreadArchive { community, thread_id, archived } =>
            map_result(chat.archive_thread(&community, &thread_id, archived).await),
        IpcRequest::GameServerAdd { community, game_id, label, address } =>
            map_result(chat.add_game_server(&community, &game_id, &label, &address).await),
        IpcRequest::GameServerRemove { community, server_id } =>
            map_result(chat.remove_game_server(&community, &server_id).await),

        // ── System ───────────────────────────────────────────────
        IpcRequest::SystemAnnounce { community, body } =>
            map_result(chat.system_message(&community, &body).await),
        IpcRequest::RaidAlert { community, active } =>
            map_result(chat.raid_alert(&community, active).await),
        IpcRequest::LockdownToggle { community, locked } =>
            map_result(chat.channel_lockdown(&community, locked).await),
        IpcRequest::KickNotify { community, target_pseudonym } =>
            map_result(chat.kicked_notification(&community, &target_pseudonym).await),
        IpcRequest::BootstrapRequest { community } =>
            map_result(chat.bootstrap_request(&community).await),
        IpcRequest::BootstrapRespond { community, target_pseudonym, governance_entries, member_list, channel_meks, recent_messages, wrapped_owner_keypair } =>
            map_result(chat.bootstrap_response(&community, &target_pseudonym, governance_entries, member_list, channel_meks, recent_messages, wrapped_owner_keypair).await),
        IpcRequest::SyncRequest { community, channel_id, since_timestamp } =>
            map_result(chat.sync_request(&community, &channel_id, since_timestamp).await),
        IpcRequest::SyncRespond { community, target_pseudonym, channel_id, messages } =>
            map_result(chat.sync_response(&community, &target_pseudonym, &channel_id, messages).await),

        // ── Identity ─────────────────────────────────────────────
        IpcRequest::IdentityShow =>
            IpcResponse::ok(&chat.identity_show()),
        IpcRequest::IdentityExport =>
            map_result(chat.identity_export()),
        IpcRequest::IdentityRotate =>
            map_result(chat.identity_rotate().await),
        IpcRequest::IdentityWipe { confirmation } =>
            map_result(chat.identity_wipe(&confirmation).await),

        // ── Friends ──────────────────────────────────────────────
        IpcRequest::FriendList =>
            IpcResponse::ok(&chat.list_friends()),

        // ── Community info/admin ─────────────────────────────────
        IpcRequest::CommunityList =>
            IpcResponse::ok(&chat.list_communities()),
        IpcRequest::CommunityInfo { governance_key } =>
            map_result(chat.community_info(&governance_key).await),
        IpcRequest::CommunityApprove { governance_key, member_pseudonym } =>
            map_result(chat.approve_member(&governance_key, &member_pseudonym).await),
        IpcRequest::CommunityReject { governance_key, member_pseudonym, reason } =>
            map_result(chat.reject_member(&governance_key, &member_pseudonym, &reason).await),
        IpcRequest::CommunityPendingMembers { governance_key } =>
            map_result(chat.pending_members(&governance_key).await),
        IpcRequest::CommunityTransferOwnership { governance_key, new_owner_pseudonym } =>
            map_result(chat.transfer_ownership(&governance_key, &new_owner_pseudonym).await),

        // ── Channels ─────────────────────────────────────────────
        IpcRequest::ChannelList { community } =>
            map_result(chat.list_channels(&community).await),
        IpcRequest::ChannelCreate { community, name, kind, .. } =>
            map_result(chat.create_channel(&community, &name, &kind).await),
        IpcRequest::ChannelDelete { community, channel_id } =>
            map_result(chat.delete_channel(&community, &channel_id).await),
        IpcRequest::ChannelUpdate { community, channel_id, name, topic, .. } =>
            map_result(chat.update_channel(&community, &channel_id, name.as_deref(), topic.as_deref()).await),
        IpcRequest::ChannelHistory { community, channel, limit } =>
            map_result(chat.channel_history(&community, &channel, limit)),
        IpcRequest::MessageEdit { community, channel, message_id, new_body } =>
            map_result(chat.edit_channel_message(&community, &channel, &message_id, &new_body).await),
        IpcRequest::MessageDelete { community, channel, message_id } =>
            map_result(chat.delete_channel_message(&community, &channel, &message_id).await),

        // ── DMs ──────────────────────────────────────────────────
        IpcRequest::DmTyping { peer_key, typing } =>
            map_result(chat.send_dm_typing(&peer_key, typing).await),
        IpcRequest::DmInbox { limit } =>
            IpcResponse::ok(&chat.dm_inbox(limit)),

        // ── Subscriptions ────────────────────────────────────────
        IpcRequest::Subscribe { .. } =>
            IpcResponse::error(400, "subscribe handled by IPC bus server"),
        IpcRequest::Unsubscribe { .. } =>
            IpcResponse::error(400, "unsubscribe handled by IPC bus server"),
        IpcRequest::MarkRead { context } => {
            match context {
                crate::ipc::protocol::ReadContext::Channel { community, channel } => {
                    chat.mark_channel_read(&community, &channel);
                    IpcResponse::ok(&serde_json::json!({"marked": "channel"}))
                }
                crate::ipc::protocol::ReadContext::Dm { peer } => {
                    chat.mark_dm_read(&peer);
                    IpcResponse::ok(&serde_json::json!({"marked": "dm"}))
                }
            }
        }

        // ── Keys / MEK ───────────────────────────────────────────
        IpcRequest::MekList { community } =>
            IpcResponse::ok(&chat.mek_list(&community)),
        IpcRequest::MekRotate { community, channel } =>
            map_result(chat.mek_rotate(&community, &channel).await),
        IpcRequest::MekRequest { community, channel, generation } =>
            map_result(chat.mek_request(&community, &channel, generation).await),
        IpcRequest::PrekeyReplenish =>
            map_result(chat.prekey_replenish().await),

        // ── Presence ─────────────────────────────────────────────
        IpcRequest::GamePresenceSet { game_name, game_id, elapsed_seconds, server_address } =>
            map_result(chat.set_game_presence(&game_name, game_id, elapsed_seconds, server_address.as_deref()).await),

        // ── Roles ────────────────────────────────────────────────
        IpcRequest::RoleList { community } =>
            map_result(chat.list_roles(&community).await),
        IpcRequest::RoleCreate { community, name, permissions, color, position } =>
            map_result(chat.create_role(&community, &name, permissions, color, position).await),
        IpcRequest::RoleUpdate { community, role_id, name, permissions, color } =>
            map_result(chat.update_role(&community, role_id, name.as_deref(), permissions, color).await),
        IpcRequest::RoleDelete { community, role_id } =>
            map_result(chat.delete_role(&community, role_id).await),
        IpcRequest::RoleAssign { community, member_pseudonym, role_id } =>
            map_result(chat.assign_role(&community, &member_pseudonym, role_id).await),
        IpcRequest::RoleUnassign { community, member_pseudonym, role_id } =>
            map_result(chat.unassign_role(&community, &member_pseudonym, role_id).await),

        // ── Moderation ───────────────────────────────────────────
        IpcRequest::Kick { community, target_pseudonym } =>
            map_result(chat.kick_member(&community, &target_pseudonym).await),
        IpcRequest::Ban { community, target_pseudonym, reason } =>
            map_result(chat.ban_member(&community, &target_pseudonym, reason.as_deref()).await),
        IpcRequest::Unban { community, target_pseudonym } =>
            map_result(chat.unban_member(&community, &target_pseudonym).await),
        IpcRequest::Timeout { community, target_pseudonym, duration_seconds, .. } =>
            map_result(chat.timeout_member(&community, &target_pseudonym, duration_seconds).await),
        IpcRequest::BanList { community } =>
            map_result(chat.list_bans(&community).await),

        // ── Invites ──────────────────────────────────────────────
        IpcRequest::InviteCreate { community, max_uses, expires_seconds } =>
            map_result(chat.create_invite(&community, max_uses, expires_seconds).await),
        IpcRequest::InviteList { community } =>
            map_result(chat.list_invites(&community).await),
        IpcRequest::InviteRevoke { community, invite_code } =>
            map_result(chat.revoke_invite(&community, &invite_code).await),

        // ── Voice ────────────────────────────────────────────────
        IpcRequest::VoiceJoin { community, channel, muted, deafened } =>
            map_result(chat.voice_join(&community, &channel, muted, deafened).await),
        IpcRequest::VoiceLeave =>
            map_result(chat.voice_leave().await),
        IpcRequest::VoiceMute { muted } =>
            map_result(chat.voice_mute(muted).await),
        IpcRequest::VoiceDeafen { deafened } =>
            map_result(chat.voice_deafen(deafened).await),

        // ── Handled in outer dispatch() ──────────────────────────
        IpcRequest::NetworkStatus | IpcRequest::NetworkPeers |
        IpcRequest::AgentRegister { .. } | IpcRequest::AgentRevoke { .. } | IpcRequest::PolicyReload |
        IpcRequest::BulkTransferStart { .. } | IpcRequest::BulkTransferComplete { .. } |
        IpcRequest::BulkTransferCancel { .. } | IpcRequest::BulkTransferStatus { .. } |
        IpcRequest::EventResume { .. } |
        IpcRequest::Status | IpcRequest::Unlock { .. } |
        IpcRequest::Lock | IpcRequest::Shutdown => unreachable!(),
    }
}

/// Convert a ChatService Result into an IpcResponse.
fn map_result<T: serde::Serialize>(result: Result<T, rekindle_chat::ChatError>) -> IpcResponse {
    match result {
        Ok(val) => IpcResponse::ok(&val),
        Err(e) => IpcResponse::error(500, format!("{e}")),
    }
}

fn state_error(state: DaemonState, required: &str) -> IpcResponse {
    IpcResponse::error_with_remediation(
        409,
        format!("cannot perform {required} in state '{}'", state.as_str()),
        if state == DaemonState::Locked {
            "unlock the daemon first: rekindle unlock"
        } else {
            "wait for the daemon to reach operational state"
        },
    )
}

impl DaemonContext {
    /// Run the daemon subscriber consuming `RoutedFrame`s from the server.
    ///
    /// The server sends `RoutedFrame` (routing header + raw bytes) via
    /// `daemon_tx`. The subscriber deserializes the raw bytes only when
    /// it needs to dispatch on the payload variant. The response is sent
    /// back through the `BusClient`'s `BusResponder` (which writes through
    /// the bus connection to the server, which routes it back to the
    /// originating client via correlation ID).
    pub async fn run_subscriber(
        self: &Arc<Self>,
        mut routed_rx: tokio::sync::mpsc::Receiver<crate::ipc::message::RoutedFrame>,
        responder: crate::ipc::client::BusResponder,
    ) {
        tracing::info!("daemon bus subscriber started (RoutedFrame path)");

        loop {
            let Some(routed) = routed_rx.recv().await else {
                tracing::info!("subscriber: routed channel closed");
                break;
            };

            // Deserialize the full message only now — at the dispatch boundary.
            let msg: crate::ipc::message::Message<crate::ipc::protocol::BusPayload> =
                match crate::ipc::framing::decode_frame(&routed.raw) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(error = %e, "subscriber: decode failed");
                        continue;
                    }
                };

            let crate::ipc::protocol::BusPayload::Request(request) = msg.payload else { continue };

            let correlation_id = routed.header.msg_id;
            let level = routed.header.security_level;
            let ctx = Arc::clone(self);
            let resp = responder.clone();

            tokio::spawn(async move {
                let response = dispatch(&ctx, request, routed.verified_sender_name.clone()).await;
                let response_bytes = match serde_json::to_vec(&response) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::error!(error = %e, "dispatch: response serialize failed");
                        return;
                    }
                };
                if let Err(e) = resp.respond(response_bytes, correlation_id, level).await {
                    tracing::error!(error = %e, "dispatch: response send failed");
                }
            });
        }

        tracing::info!("daemon bus subscriber stopped");
    }
}
