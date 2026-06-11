//! Service dispatcher: thin router from daemon requests to ChatService methods.
//!
//! Every `DaemonRequest` variant maps to exactly one `ChatService` method call.
//! No business logic. No crypto. No persistence. Single-expression match arms.
//!
//! Lifecycle operations (Status, Unlock, Lock, Shutdown) are handled directly
//! because they manage the daemon state machine, vault open/close, and transport
//! start/stop — concerns that belong to the daemon, not the chat application.

pub(crate) mod admin;
pub(crate) mod bulk_transfers;
pub(crate) mod lifecycle;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use rekindle_transport_ipc::v3::bulk::counters::BulkCounters;
use rekindle_types::daemon::{AgentRegistration, DaemonRequest, DaemonResponse, ReadContext};
use rekindle_types::display::StatusSnapshot;
use rekindle_types::transport::Transport;

use crate::daemon::{DaemonLifecycle, DaemonState};
use crate::idempotency::IdempotencyCache;
use crate::journal::EventJournal;
use crate::state::StatePaths;
use crate::subscriptions::SubscriptionRegistry;

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
/// transport-ipc owns all transport plumbing (rayon pools, buffer pools,
/// bulk counters, connection state). DaemonContext holds only application
/// state that the dispatch layer needs.
pub struct DaemonContext {
    /// The chat application service. `None` before unlock.
    pub chat: RwLock<Option<Arc<rekindle_chat::ChatService>>>,

    /// The transport (`Arc<dyn Transport>`). Set during unlock.
    pub transport: RwLock<Option<Arc<dyn Transport>>>,

    /// The vault store. Set during unlock.
    pub vault: RwLock<Option<Arc<rekindle_storage::VaultStore>>>,

    /// Current daemon lifecycle state.
    pub lifecycle: Arc<DaemonLifecycle>,

    /// Resolved XDG paths.
    pub paths: StatePaths,

    /// Active authorization policy.
    pub policy: RwLock<PolicyConfig>,

    /// Cached status snapshot with TTL.
    pub status_cache: Mutex<Option<(Instant, StatusSnapshot)>>,

    /// Idempotency cache for exactly-once request processing.
    pub idempotency_cache: IdempotencyCache,

    /// Shared event journal for cursor-based resumption.
    pub event_journal: Arc<EventJournal>,

    /// Per-connection subscription registry for event fan-out.
    pub subscriptions: Arc<SubscriptionRegistry>,

    /// Transport-layer observability counters from transport-ipc.
    /// Shared Arc — transport-ipc writes, node reads for status/metrics/diagnostics.
    pub transport_counters: Arc<BulkCounters>,

    /// Registered agents — application-level identity metadata.
    pub agents: RwLock<HashMap<String, AgentRegistration>>,

    /// Per-transfer lifecycle tracking for `rekindle transfer status`.
    pub bulk_transfers: Mutex<bulk_transfers::BulkTransferRegistry>,
}

/// Dispatch a daemon request to the appropriate handler.
pub async fn dispatch(ctx: &Arc<DaemonContext>, request: DaemonRequest, sender_name: Option<Arc<str>>) -> DaemonResponse {
    let state = ctx.lifecycle.state();
    let sender = sender_name.as_deref().unwrap_or("anonymous");
    tracing::debug!(sender, state = state.as_str(), request = ?request, "dispatch");

    match request {
        // ── Lifecycle (any state) ────────────────────────────────
        DaemonRequest::Status => lifecycle::handle_status(ctx, state),
        DaemonRequest::NetworkStatus => admin::handle_network_status(ctx, state),
        DaemonRequest::NetworkPeers => admin::handle_network_peers(ctx, state),
        DaemonRequest::Unlock { passphrase } => {
            lifecycle::handle_unlock(ctx, state, &passphrase).await
        }
        DaemonRequest::Lock => lifecycle::handle_lock(ctx, state).await,
        DaemonRequest::Shutdown => lifecycle::handle_shutdown(ctx, state).await,
        DaemonRequest::AgentRegister { name, agent_type, capabilities } =>
            admin::handle_agent_register(ctx, &name, agent_type, &capabilities),
        DaemonRequest::AgentRevoke { name } =>
            admin::handle_agent_revoke(ctx, &name),
        DaemonRequest::PolicyReload => admin::handle_policy_reload(ctx),

        // ── Bulk Transfer (control-plane signaling) ──────────────
        DaemonRequest::BulkTransferStart { transfer_id, total_size, media_type, digest, direction } => {
            let stream_id = ctx.bulk_transfers.lock().start(
                transfer_id.clone(), total_size, media_type, digest, direction, 0,
            );
            tracing::info!(transfer_id, total_size, stream_id, "bulk transfer started");
            DaemonResponse::ok(&serde_json::json!({
                "transfer_id": transfer_id,
                "stream_id": stream_id,
                "status": "active",
            }))
        }
        DaemonRequest::BulkTransferComplete { transfer_id, digest, bytes_transferred } => {
            let found = ctx.bulk_transfers.lock().complete(&transfer_id, bytes_transferred);
            tracing::info!(transfer_id, digest, bytes_transferred, found, "bulk transfer completed");
            if found {
                DaemonResponse::ok(&serde_json::json!({
                    "transfer_id": transfer_id,
                    "status": "completed",
                }))
            } else {
                DaemonResponse::error(404, format!("transfer {transfer_id} not found"))
            }
        }
        DaemonRequest::BulkTransferCancel { transfer_id, reason } => {
            let found = ctx.bulk_transfers.lock().cancel(&transfer_id);
            tracing::info!(transfer_id, reason, found, "bulk transfer cancelled");
            if found {
                DaemonResponse::ok(&serde_json::json!({
                    "transfer_id": transfer_id,
                    "status": "cancelled",
                }))
            } else {
                DaemonResponse::error(404, format!("transfer {transfer_id} not found"))
            }
        }
        DaemonRequest::BulkTransferStatus { transfer_id } => {
            match ctx.bulk_transfers.lock().status(&transfer_id) {
                Some(state) => DaemonResponse::ok(&state),
                None => DaemonResponse::error(404, format!("transfer {transfer_id} not found")),
            }
        }

        // ── Event Journal Resume ─────────────────────────────────
        DaemonRequest::EventResume { last_seen_seq } => {
            match ctx.event_journal.replay_from(last_seen_seq) {
                Ok(events) => {
                    let event_data: Vec<serde_json::Value> = events
                        .iter()
                        .map(|e| serde_json::json!({
                            "seq": e.seq,
                            "event": *e.event,
                        }))
                        .collect();
                    DaemonResponse::ok(&serde_json::json!({
                        "replayed": events.len(),
                        "current_head": ctx.event_journal.head_seq(),
                        "events": event_data,
                    }))
                }
                Err(e) => DaemonResponse::error_with_remediation(
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
                return DaemonResponse::error(503, "chat service not initialized — unlock first");
            };

            let client_msg_id = match &request {
                DaemonRequest::ChannelSend { client_msg_id: Some(id), .. } => {
                    if let Some(cached) = ctx.idempotency_cache.check(id) {
                        return cached;
                    }
                    Some(id.clone())
                }
                _ => None,
            };

            let response = dispatch_to_chat(&chat, request, state).await;

            if let Some(id) = client_msg_id {
                ctx.idempotency_cache.store(id, response.clone());
            }

            response
        }
    }
}

/// Forward business logic requests to ChatService methods.
async fn dispatch_to_chat(
    chat: &rekindle_chat::ChatService,
    request: DaemonRequest,
    _state: DaemonState,
) -> DaemonResponse {
    match request {
        // ── Identity ─────────────────────────────────────────────
        DaemonRequest::IdentityCreate { display_name } =>
            map_result(chat.init_identity(&display_name).await),
        DaemonRequest::IdentityDestroy { .. } =>
            map_result(chat.destroy_identity().await),
        DaemonRequest::IdentityExportEncrypted { passphrase } =>
            map_result(chat.identity_export_encrypted(&passphrase)),
        DaemonRequest::IdentityImportEncrypted { passphrase, data } =>
            map_result(chat.identity_import_encrypted(&passphrase, &data)),
        DaemonRequest::IdentityImport { data } =>
            map_result(chat.identity_import(&data)),
        DaemonRequest::IdentityShow =>
            DaemonResponse::ok(&chat.identity_show()),
        DaemonRequest::IdentityExport =>
            map_result(chat.identity_export()),
        DaemonRequest::IdentityRotate =>
            map_result(chat.identity_rotate().await),
        DaemonRequest::IdentityWipe { confirmation } =>
            map_result(chat.identity_wipe(&confirmation).await),

        // ── Community ────────────────────────────────────────────
        DaemonRequest::CommunityCreate { name, description } =>
            map_result(chat.create_community(&name, &description).await),
        DaemonRequest::CommunityJoin { invite } =>
            map_result(chat.join_community(&invite).await),
        DaemonRequest::CommunityLeave { governance_key } =>
            map_result(chat.leave_community(&governance_key).await),
        DaemonRequest::CommunityList =>
            DaemonResponse::ok(&chat.list_communities()),
        DaemonRequest::CommunityInfo { governance_key } =>
            map_result(chat.community_info(&governance_key).await),
        DaemonRequest::CommunityApprove { governance_key, member_pseudonym } =>
            map_result(chat.approve_member(&governance_key, &member_pseudonym).await),
        DaemonRequest::CommunityReject { governance_key, member_pseudonym, reason } =>
            map_result(chat.reject_member(&governance_key, &member_pseudonym, &reason).await),
        DaemonRequest::CommunityPendingMembers { governance_key } =>
            map_result(chat.pending_members(&governance_key).await),
        DaemonRequest::CommunityTransferOwnership { governance_key, new_owner_pseudonym } =>
            map_result(chat.transfer_ownership(&governance_key, &new_owner_pseudonym).await),

        // ── Channel ──────────────────────────────────────────────
        DaemonRequest::ChannelList { community } =>
            map_result(chat.list_channels(&community).await),
        DaemonRequest::ChannelCreate { community, name, kind, .. } =>
            map_result(chat.create_channel(&community, &name, &kind).await),
        DaemonRequest::ChannelDelete { community, channel_id } =>
            map_result(chat.delete_channel(&community, &channel_id).await),
        DaemonRequest::ChannelUpdate { community, channel_id, name, topic, .. } =>
            map_result(chat.update_channel(&community, &channel_id, name.as_deref(), topic.as_deref()).await),
        DaemonRequest::ChannelSend { community, channel, body, reply_to, .. } =>
            map_result(chat.send_channel_message(&community, &channel, &body, reply_to).await),
        DaemonRequest::ChannelTyping { community, channel } =>
            map_result(chat.send_channel_typing(&community, &channel).await),
        DaemonRequest::ChannelHistory { community, channel, limit } =>
            map_result(chat.channel_history(&community, &channel, limit)),
        DaemonRequest::MessageEdit { community, channel, message_id, new_body } =>
            map_result(chat.edit_channel_message(&community, &channel, &message_id, &new_body).await),
        DaemonRequest::MessageDelete { community, channel, message_id } =>
            map_result(chat.delete_channel_message(&community, &channel, &message_id).await),

        // ── DMs ──────────────────────────────────────────────────
        DaemonRequest::DmSend { peer_key, body } =>
            map_result(chat.send_dm(&peer_key, &body).await),
        DaemonRequest::DmTyping { peer_key, typing } =>
            map_result(chat.send_dm_typing(&peer_key, typing).await),
        DaemonRequest::DmInbox { limit } =>
            DaemonResponse::ok(&chat.dm_inbox(limit)),
        DaemonRequest::DmThread { peer_key, limit } =>
            map_result(chat.dm_thread(&peer_key, limit)),

        // ── Friends ──────────────────────────────────────────────
        DaemonRequest::FriendAdd { target_profile_key, message } =>
            map_result(chat.send_friend_request(&target_profile_key, &message).await),
        DaemonRequest::FriendAccept { public_key } =>
            map_result(chat.accept_friend_request(&public_key).await),
        DaemonRequest::FriendReject { public_key } =>
            map_result(chat.reject_friend_request(&public_key).await),
        DaemonRequest::FriendRemove { public_key } =>
            map_result(chat.remove_friend(&public_key).await),
        DaemonRequest::FriendList =>
            DaemonResponse::ok(&chat.list_friends()),
        DaemonRequest::FriendRequests =>
            DaemonResponse::ok(&chat.list_pending_requests()),

        // ── Subscriptions (handled server-side in DaemonRouter) ──
        DaemonRequest::Subscribe { .. } =>
            DaemonResponse::error(400, "subscribe handled by DaemonRouter"),
        DaemonRequest::Unsubscribe { .. } =>
            DaemonResponse::error(400, "unsubscribe handled by DaemonRouter"),
        DaemonRequest::MarkRead { context } => {
            match context {
                ReadContext::Channel { community, channel } => {
                    chat.mark_channel_read(&community, &channel);
                    DaemonResponse::ok(&serde_json::json!({"marked": "channel"}))
                }
                ReadContext::Dm { peer } => {
                    chat.mark_dm_read(&peer);
                    DaemonResponse::ok(&serde_json::json!({"marked": "dm"}))
                }
            }
        }

        // ── Keys / MEK ───────────────────────────────────────────
        DaemonRequest::MekList { community } =>
            DaemonResponse::ok(&chat.mek_list(&community)),
        DaemonRequest::MekRotate { community, channel } =>
            map_result(chat.mek_rotate(&community, &channel).await),
        DaemonRequest::MekRequest { community, channel, generation } =>
            map_result(chat.mek_request(&community, &channel, generation).await),
        DaemonRequest::PrekeyReplenish =>
            map_result(chat.prekey_replenish().await),

        // ── Presence ─────────────────────────────────────────────
        DaemonRequest::PresenceSet { status, message } =>
            map_result(chat.set_presence(&status, message.as_deref()).await),
        DaemonRequest::GamePresenceSet { game_name, game_id, elapsed_seconds, server_address } =>
            map_result(chat.set_game_presence(&game_name, game_id, elapsed_seconds, server_address.as_deref()).await),
        DaemonRequest::GamePresenceClear =>
            map_result(chat.set_presence("online", None).await),

        // ── Roles ────────────────────────────────────────────────
        DaemonRequest::RoleList { community } =>
            map_result(chat.list_roles(&community).await),
        DaemonRequest::RoleCreate { community, name, permissions, color, position } =>
            map_result(chat.create_role(&community, &name, permissions, color, position).await),
        DaemonRequest::RoleUpdate { community, role_id, name, permissions, color } =>
            map_result(chat.update_role(&community, role_id, name.as_deref(), permissions, color).await),
        DaemonRequest::RoleDelete { community, role_id } =>
            map_result(chat.delete_role(&community, role_id).await),
        DaemonRequest::RoleAssign { community, member_pseudonym, role_id } =>
            map_result(chat.assign_role(&community, &member_pseudonym, role_id).await),
        DaemonRequest::RoleUnassign { community, member_pseudonym, role_id } =>
            map_result(chat.unassign_role(&community, &member_pseudonym, role_id).await),

        // ── Moderation ───────────────────────────────────────────
        DaemonRequest::Kick { community, target_pseudonym } =>
            map_result(chat.kick_member(&community, &target_pseudonym).await),
        DaemonRequest::Ban { community, target_pseudonym, reason } =>
            map_result(chat.ban_member(&community, &target_pseudonym, reason.as_deref()).await),
        DaemonRequest::Unban { community, target_pseudonym } =>
            map_result(chat.unban_member(&community, &target_pseudonym).await),
        DaemonRequest::Timeout { community, target_pseudonym, duration_seconds, .. } =>
            map_result(chat.timeout_member(&community, &target_pseudonym, duration_seconds).await),
        DaemonRequest::BanList { community } =>
            map_result(chat.list_bans(&community).await),

        // ── Invites ──────────────────────────────────────────────
        DaemonRequest::InviteCreate { community, max_uses, expires_seconds } =>
            map_result(chat.create_invite(&community, max_uses, expires_seconds).await),
        DaemonRequest::InviteList { community } =>
            map_result(chat.list_invites(&community).await),
        DaemonRequest::InviteRevoke { community, invite_code } =>
            map_result(chat.revoke_invite(&community, &invite_code).await),

        // ── Social ────────────────────────────────────────────────
        DaemonRequest::ReactionAdd { community, channel, message_id, emoji } =>
            map_result(chat.add_reaction(&community, &channel, &message_id, &emoji).await),
        DaemonRequest::ReactionRemove { community, channel, message_id, emoji } =>
            map_result(chat.remove_reaction(&community, &channel, &message_id, &emoji).await),
        DaemonRequest::PinAdd { community, channel, message_id } =>
            map_result(chat.pin_message(&community, &channel, &message_id).await),
        DaemonRequest::PinRemove { community, channel, message_id } =>
            map_result(chat.unpin_message(&community, &channel, &message_id).await),
        DaemonRequest::EventCreate { community, title, description, start_time, end_time, channel_id, max_attendees } =>
            map_result(chat.create_event(&community, &title, &description, start_time, end_time, channel_id.as_deref(), max_attendees).await),
        DaemonRequest::EventUpdate { community, event_id, title, description, start_time, end_time, max_attendees } =>
            map_result(chat.update_event(&community, &event_id, &title, &description, start_time, end_time, max_attendees).await),
        DaemonRequest::EventDelete { community, event_id } =>
            map_result(chat.delete_event(&community, &event_id).await),
        DaemonRequest::EventRsvp { community, event_id, status } =>
            map_result(chat.rsvp_event(&community, &event_id, &status).await),
        DaemonRequest::EventRemind { community, event_id, title, minutes_until } =>
            map_result(chat.event_reminder(&community, &event_id, &title, minutes_until).await),
        DaemonRequest::ThreadCreate { community, channel, parent_message_id, title, auto_archive_seconds } =>
            map_result(chat.create_thread(&community, &channel, &parent_message_id, &title, auto_archive_seconds).await),
        DaemonRequest::ThreadMessage { community, thread_id, ciphertext, mek_generation, reply_to_id } =>
            map_result(chat.thread_message(&community, &thread_id, ciphertext, mek_generation, reply_to_id.as_deref()).await),
        DaemonRequest::ThreadArchive { community, thread_id, archived } =>
            map_result(chat.archive_thread(&community, &thread_id, archived).await),
        DaemonRequest::GameServerAdd { community, game_id, label, address } =>
            map_result(chat.add_game_server(&community, &game_id, &label, &address).await),
        DaemonRequest::GameServerRemove { community, server_id } =>
            map_result(chat.remove_game_server(&community, &server_id).await),

        // ── System ───────────────────────────────────────────────
        DaemonRequest::SystemAnnounce { community, body } =>
            map_result(chat.system_message(&community, &body).await),
        DaemonRequest::RaidAlert { community, active } =>
            map_result(chat.raid_alert(&community, active).await),
        DaemonRequest::LockdownToggle { community, locked } =>
            map_result(chat.channel_lockdown(&community, locked).await),
        DaemonRequest::KickNotify { community, target_pseudonym } =>
            map_result(chat.kicked_notification(&community, &target_pseudonym).await),
        DaemonRequest::BootstrapRequest { community } =>
            map_result(chat.bootstrap_request(&community).await),
        DaemonRequest::BootstrapRespond { community, target_pseudonym, governance_entries, member_list, channel_meks, recent_messages, wrapped_owner_keypair } =>
            map_result(chat.bootstrap_response(&community, &target_pseudonym, governance_entries, member_list, channel_meks, recent_messages, wrapped_owner_keypair).await),
        DaemonRequest::SyncRequest { community, channel_id, since_timestamp } =>
            map_result(chat.sync_request(&community, &channel_id, since_timestamp).await),
        DaemonRequest::SyncRespond { community, target_pseudonym, channel_id, messages } =>
            map_result(chat.sync_response(&community, &target_pseudonym, &channel_id, messages).await),

        // ── Voice ────────────────────────────────────────────────
        DaemonRequest::VoiceJoin { community, channel, muted, deafened } =>
            map_result(chat.voice_join(&community, &channel, muted, deafened).await),
        DaemonRequest::VoiceLeave =>
            map_result(chat.voice_leave().await),
        DaemonRequest::VoiceMute { muted } =>
            map_result(chat.voice_mute(muted).await),
        DaemonRequest::VoiceDeafen { deafened } =>
            map_result(chat.voice_deafen(deafened).await),

        // ── Handled in outer dispatch() ──────────────────────────
        DaemonRequest::NetworkStatus | DaemonRequest::NetworkPeers |
        DaemonRequest::AgentRegister { .. } | DaemonRequest::AgentRevoke { .. } | DaemonRequest::PolicyReload |
        DaemonRequest::BulkTransferStart { .. } | DaemonRequest::BulkTransferComplete { .. } |
        DaemonRequest::BulkTransferCancel { .. } | DaemonRequest::BulkTransferStatus { .. } |
        DaemonRequest::EventResume { .. } |
        DaemonRequest::Status | DaemonRequest::Unlock { .. } |
        DaemonRequest::Lock | DaemonRequest::Shutdown => unreachable!(),
    }
}

/// Convert a ChatService Result into a DaemonResponse.
fn map_result<T: serde::Serialize>(result: Result<T, rekindle_chat::ChatError>) -> DaemonResponse {
    match result {
        Ok(val) => DaemonResponse::ok(&val),
        Err(e) => DaemonResponse::error(500, format!("{e}")),
    }
}

fn state_error(state: DaemonState, required: &str) -> DaemonResponse {
    DaemonResponse::error_with_remediation(
        409,
        format!("cannot perform {required} in state '{}'", state.as_str()),
        if state == DaemonState::Locked {
            "unlock the daemon first: rekindle unlock"
        } else {
            "wait for the daemon to reach operational state"
        },
    )
}
