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

pub(crate) mod lifecycle;

use std::sync::Arc;

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
}

/// Dispatch an IPC request to the appropriate handler.
pub async fn dispatch(ctx: &Arc<DaemonContext>, request: IpcRequest) -> IpcResponse {
    let state = ctx.lifecycle.state();

    match request {
        // ── Lifecycle (any state) ────────────────────────────────
        IpcRequest::Status | IpcRequest::NetworkStatus | IpcRequest::NetworkPeers =>
            lifecycle::handle_status(ctx, state),
        IpcRequest::Unlock { passphrase } => {
            lifecycle::handle_unlock(ctx, state, &passphrase).await
        }
        IpcRequest::Lock => lifecycle::handle_lock(ctx, state).await,
        IpcRequest::Shutdown => lifecycle::handle_shutdown(ctx, state).await,

        // ── Everything else requires OPERATIONAL state + ChatService ──
        _ => {
            if !state.can_query() {
                return state_error(state, "query");
            }
            // Clone Arc from RwLock, drop guard immediately.
            let chat = ctx.chat.read().clone();
            let Some(chat) = chat else {
                return IpcResponse::error(503, "chat service not initialized — unlock first");
            };
            dispatch_to_chat(&chat, request, state).await
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
        IpcRequest::ChannelSend { community, channel, body, reply_to } =>
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

        // ── Agent Management (daemon-level) ──────────────────────
        IpcRequest::AgentRegister { .. } | IpcRequest::AgentRevoke { .. } | IpcRequest::PolicyReload =>
            IpcResponse::error(400, "agent/policy commands handled by admin dispatch"),

        // ── Handled in outer dispatch() ──────────────────────────
        IpcRequest::NetworkStatus | IpcRequest::NetworkPeers |
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
    /// Run the daemon as a bus subscriber.
    pub async fn run_subscriber(
        self: &Arc<Self>,
        mut client: crate::ipc::client::BusClient,
    ) {
        tracing::info!("daemon bus subscriber started");
        let responder = client.responder();

        loop {
            let msg = match client.recv_bus_message().await {
                Some(Ok(msg)) => msg,
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "subscriber: decode failed");
                    continue;
                }
                None => {
                    tracing::info!("subscriber: bus connection closed");
                    break;
                }
            };

            let crate::ipc::protocol::BusPayload::Request(request) = msg.payload else { continue };

            let correlation_id = msg.msg_id;
            let level = msg.security_level;
            let ctx = Arc::clone(self);
            let resp = responder.clone();

            tokio::spawn(async move {
                let response = dispatch(&ctx, request).await;
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
