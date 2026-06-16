//! Service dispatcher: single exhaustive router from DaemonRequest to handlers.
//!
//! One function, one match, two arms: `Lifecycle(l)` and `Chat(c)`.
//! Each inner match is exhaustive over its sub-enum. No `_ =>`, no
//! `unreachable!()`. Adding a new variant to either sub-enum produces
//! a compile error here — not a runtime panic.
//!
//! Lifecycle operations (Status, Unlock, Lock, Shutdown, bulk transfers,
//! agent management, event journal, subscriptions) are handled directly
//! because they manage the daemon state machine, not the chat application.
//!
//! Chat operations require OPERATIONAL state + ChatService.

pub(crate) mod admin;
pub(crate) mod bulk_transfers;
pub(crate) mod lifecycle;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use rekindle_transport_ipc::v3::bulk::counters::BulkCounters;
use rekindle_types::daemon::{AgentRegistration, ChatRequest, DaemonRequest, DaemonResponse, LifecycleRequest, ReadContext};
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
///
/// Two arms, both exhaustive. The compiler enforces that every variant
/// in both `LifecycleRequest` and `ChatRequest` is handled.
pub async fn dispatch(ctx: &Arc<DaemonContext>, request: DaemonRequest, sender_name: Option<Arc<str>>) -> DaemonResponse {
    let state = ctx.lifecycle.state();
    let sender = sender_name.as_deref().unwrap_or("anonymous");
    tracing::debug!(sender, state = state.as_str(), request = ?request, "dispatch");

    match request {
        // ══════════════════════════════════════════════════════════
        // Lifecycle — no ChatService needed, any daemon state
        // ══════════════════════════════════════════════════════════
        DaemonRequest::Lifecycle(lifecycle_req) => match lifecycle_req {
            LifecycleRequest::Status => lifecycle::handle_status(ctx, state),
            LifecycleRequest::NetworkStatus => admin::handle_network_status(ctx, state),
            LifecycleRequest::NetworkPeers => admin::handle_network_peers(ctx, state),
            LifecycleRequest::Unlock { passphrase } => {
                lifecycle::handle_unlock(ctx, state, &passphrase).await
            }
            LifecycleRequest::Lock => lifecycle::handle_lock(ctx, state).await,
            LifecycleRequest::Shutdown => lifecycle::handle_shutdown(ctx, state).await,
            LifecycleRequest::AgentRegister { name, agent_type, capabilities } =>
                admin::handle_agent_register(ctx, &name, agent_type, &capabilities),
            LifecycleRequest::AgentRevoke { name } =>
                admin::handle_agent_revoke(ctx, &name),
            LifecycleRequest::PolicyReload => admin::handle_policy_reload(ctx),

            // ── Bulk Transfer (control-plane signaling) ──────────
            LifecycleRequest::BulkTransferStart { transfer_id, total_size, media_type, digest, direction } => {
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
            LifecycleRequest::BulkTransferComplete { transfer_id, digest, bytes_transferred } => {
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
            LifecycleRequest::BulkTransferCancel { transfer_id, reason } => {
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
            LifecycleRequest::BulkTransferStatus { transfer_id } => {
                match ctx.bulk_transfers.lock().status(&transfer_id) {
                    Some(state) => DaemonResponse::ok(&state),
                    None => DaemonResponse::error(404, format!("transfer {transfer_id} not found")),
                }
            }

            // ── Event Journal Resume ─────────────────────────────
            LifecycleRequest::EventResume { last_seen_seq } => {
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

            // ── Subscriptions (handled server-side in DaemonRouter,
            //    but if they reach dispatch, return an error) ─────
            LifecycleRequest::Subscribe { .. } =>
                DaemonResponse::error(400, "subscribe handled by DaemonRouter"),
            LifecycleRequest::Unsubscribe { .. } =>
                DaemonResponse::error(400, "unsubscribe handled by DaemonRouter"),
        },

        // ══════════════════════════════════════════════════════════
        // Chat — requires OPERATIONAL state + ChatService
        // ══════════════════════════════════════════════════════════
        DaemonRequest::Chat(chat_req) => {
            if !state.can_query() {
                return state_error(state, "query");
            }
            let chat = ctx.chat.read().clone();
            let Some(chat) = chat else {
                return DaemonResponse::error(503, "chat service not initialized — unlock first");
            };

            let client_msg_id = match &chat_req {
                ChatRequest::ChannelSend { client_msg_id: Some(id), .. } => {
                    if let Some(cached) = ctx.idempotency_cache.check(id) {
                        return cached;
                    }
                    Some(id.clone())
                }
                _ => None,
            };

            let response = dispatch_chat(&chat, chat_req).await;

            if let Some(id) = client_msg_id {
                ctx.idempotency_cache.store(id, response.clone());
            }

            response
        }
    }
}

/// Forward business logic requests to ChatService methods.
///
/// Every `ChatRequest` variant maps to exactly one `ChatService` method.
/// No business logic. No crypto. No persistence. Single-expression match arms
/// plus emit_local calls for subscription event visibility.
async fn dispatch_chat(
    chat: &rekindle_chat::ChatService,
    request: ChatRequest,
) -> DaemonResponse {
    match request {
        // ── Identity ─────────────────────────────────────────────
        ChatRequest::IdentityCreate { display_name } => {
            let result = chat.init_identity(&display_name).await;
            if let Ok(ref created) = result {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::IdentityCreated {
                        public_key: created.public_key.to_hex(),
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::IdentityDestroy { .. } =>
            map_result(chat.destroy_identity().await),
        ChatRequest::IdentityExportEncrypted { passphrase } =>
            map_result(chat.identity_export_encrypted(&passphrase)),
        ChatRequest::IdentityImportEncrypted { passphrase, data } =>
            map_result(chat.identity_import_encrypted(&passphrase, &data)),
        ChatRequest::IdentityImport { data } =>
            map_result(chat.identity_import(&data)),
        ChatRequest::IdentityShow =>
            DaemonResponse::ok(&chat.identity_show()),
        ChatRequest::IdentityExport =>
            map_result(chat.identity_export()),
        ChatRequest::IdentityRotate => {
            let result = chat.identity_rotate().await;
            if result.is_ok() {
                if let Some(id) = chat.session_identity() {
                    chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::System(
                        rekindle_types::subscription_events::SystemEvent::IdentityRotated {
                            new_public_key: id.public_key.to_hex(),
                        },
                    ));
                }
            }
            map_result(result)
        }
        ChatRequest::IdentityWipe { confirmation } =>
            map_result(chat.identity_wipe(&confirmation).await),

        // ── Community ────────────────────────────────────────────
        ChatRequest::CommunityCreate { name, description } => {
            let result = chat.create_community(&name, &description).await;
            if let Ok(ref created) = result {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Membership(
                    rekindle_types::subscription_events::MembershipEvent::Created {
                        community: created.community_name.clone(),
                        governance_key: created.governance_key.clone(),
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::CommunityJoin { invite } => {
            let result = chat.join_community(&invite).await;
            if let Ok(ref joined) = result {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Membership(
                    rekindle_types::subscription_events::MembershipEvent::CommunityJoined {
                        community: joined.community_name.clone(),
                        governance_key: joined.governance_key.clone(),
                        slot_index: joined.slot_index,
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::CommunityLeave { governance_key } => {
            let result = chat.leave_community(&governance_key).await;
            if result.is_ok() {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Membership(
                    rekindle_types::subscription_events::MembershipEvent::CommunityLeft {
                        community: governance_key.clone(),
                        governance_key,
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::CommunityList =>
            DaemonResponse::ok(&chat.list_communities()),
        ChatRequest::CommunityInfo { governance_key } =>
            map_result(chat.community_info(&governance_key).await),
        ChatRequest::CommunityApprove { governance_key, member_pseudonym } =>
            map_result(chat.approve_member(&governance_key, &member_pseudonym).await),
        ChatRequest::CommunityReject { governance_key, member_pseudonym, reason } =>
            map_result(chat.reject_member(&governance_key, &member_pseudonym, &reason).await),
        ChatRequest::CommunityPendingMembers { governance_key } =>
            map_result(chat.pending_members(&governance_key).await),
        ChatRequest::CommunityTransferOwnership { governance_key, new_owner_pseudonym } =>
            map_result(chat.transfer_ownership(&governance_key, &new_owner_pseudonym).await),

        // ── Channel ──────────────────────────────────────────────
        ChatRequest::ChannelList { community } =>
            map_result(chat.list_channels(&community).await),
        ChatRequest::ChannelCreate { community, name, kind, .. } => {
            let result = chat.create_channel(&community, &name, &kind).await;
            if result.is_ok() {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Governance(
                    rekindle_types::subscription_events::GovernanceEvent::ChannelsChanged {
                        community: community.clone(),
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::ChannelDelete { community, channel_id } => {
            let result = chat.delete_channel(&community, &channel_id).await;
            if result.is_ok() {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Governance(
                    rekindle_types::subscription_events::GovernanceEvent::ChannelsChanged {
                        community: community.clone(),
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::ChannelUpdate { community, channel_id, name, topic, .. } =>
            map_result(chat.update_channel(&community, &channel_id, name.as_deref(), topic.as_deref()).await),
        ChatRequest::ChannelSend { community, channel, body, reply_to, .. } => {
            let result = chat.send_channel_message(&community, &channel, &body, reply_to).await;
            if let Ok(ref sent) = result {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
                    rekindle_types::subscription_events::ChannelMessageEvent::New {
                        community: community.clone(),
                        channel: channel.clone(),
                        message_id: sent.message_id.clone(),
                        sender_pseudonym: String::new(),
                        sequence: 0,
                        timestamp: sent.timestamp,
                        body: Some(body.clone()),
                        reply_to_sequence: None,
                        is_self: true,
                        client_msg_id: None,
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::ChannelTyping { community, channel } =>
            map_result(chat.send_channel_typing(&community, &channel).await),
        ChatRequest::ChannelHistory { community, channel, limit } =>
            map_result(chat.channel_history(&community, &channel, limit)),
        ChatRequest::MessageEdit { community, channel, message_id, new_body } => {
            let result = chat.edit_channel_message(&community, &channel, &message_id, &new_body).await;
            if result.is_ok() {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
                    rekindle_types::subscription_events::ChannelMessageEvent::Edited {
                        community: community.clone(),
                        channel,
                        message_id,
                        edited_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        body: Some(new_body),
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::MessageDelete { community, channel, message_id } => {
            let result = chat.delete_channel_message(&community, &channel, &message_id).await;
            if result.is_ok() {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
                    rekindle_types::subscription_events::ChannelMessageEvent::Deleted {
                        community,
                        channel,
                        message_id,
                    },
                ));
            }
            map_result(result)
        }

        // ── DMs ──────────────────────────────────────────────────
        ChatRequest::DmSend { peer_key, body } => {
            // dm/sender.rs emits DirectMessageReceived internally — no local emit here.
            map_result(chat.send_dm(&peer_key, &body).await)
        }
        ChatRequest::DmTyping { peer_key, typing } =>
            map_result(chat.send_dm_typing(&peer_key, typing).await),
        ChatRequest::DmInbox { limit } =>
            map_result(chat.dm_inbox(limit)),
        ChatRequest::DmThread { peer_key, limit } =>
            map_result(chat.dm_thread(&peer_key, limit)),
        ChatRequest::DmStart { peer_key, pseudonym, is_group } =>
            map_result(chat.start_dm(&peer_key, &pseudonym, is_group).await),
        ChatRequest::DmAccept { record_key } =>
            map_result(chat.accept_dm(&record_key).await),

        // ── Friends ──────────────────────────────────────────────
        ChatRequest::FriendAdd { target_profile_key, message } => {
            let result = chat.send_friend_request(&target_profile_key, &message).await;
            if let Ok(ref sent) = result {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Friend(
                    rekindle_types::subscription_events::FriendEvent::RequestSent {
                        target_profile_key: target_profile_key.clone(),
                        dm_log_key: sent.dm_log_key.clone(),
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::FriendAccept { public_key } => {
            let result = chat.accept_friend_request(&public_key).await;
            if let Ok(ref accepted) = result {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Friend(
                    rekindle_types::subscription_events::FriendEvent::Accepted {
                        peer_key: accepted.peer_key.clone(),
                        dm_log_key: accepted.outbound_log.clone(),
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::FriendReject { public_key } => {
            let result = chat.reject_friend_request(&public_key).await;
            if result.is_ok() {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Friend(
                    rekindle_types::subscription_events::FriendEvent::Rejected {
                        peer_key: public_key,
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::FriendRemove { public_key } => {
            let result = chat.remove_friend(&public_key).await;
            if result.is_ok() {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Friend(
                    rekindle_types::subscription_events::FriendEvent::Removed {
                        peer_key: public_key,
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::FriendList =>
            DaemonResponse::ok(&chat.list_friends()),
        ChatRequest::FriendRequests =>
            DaemonResponse::ok(&chat.list_pending_requests()),

        // ── Read State ──────────────────────────────────────────
        ChatRequest::MarkRead { context } => {
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
        ChatRequest::MekList { community } =>
            DaemonResponse::ok(&chat.mek_list(&community)),
        ChatRequest::MekRotate { community, channel } => {
            let result = chat.mek_rotate(&community, &channel).await;
            if let Ok(ref generation) = result {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Crypto(
                    rekindle_types::subscription_events::CryptoEvent::MekRotated {
                        community: community.clone(),
                        channel: Some(channel.clone()),
                        generation: *generation,
                        rotator_pseudonym: None,
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::MekRequest { community, channel, generation } =>
            map_result(chat.mek_request(&community, &channel, generation).await),
        ChatRequest::PrekeyReplenish =>
            map_result(chat.prekey_replenish().await),

        // ── Presence ─────────────────────────────────────────────
        ChatRequest::PresenceSet { status, message } => {
            let result = chat.set_presence(&status, message.as_deref()).await;
            if result.is_ok() {
                for community in chat.list_communities() {
                    chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Presence(
                        rekindle_types::subscription_events::PresenceEvent::CommunityMemberChanged {
                            community: community.governance_key,
                            pseudonym: community.pseudonym,
                            status: status.clone(),
                            game_name: None,
                            game_id: None,
                        },
                    ));
                }
            }
            map_result(result)
        }
        ChatRequest::GamePresenceSet { game_name, game_id, elapsed_seconds, server_address } => {
            let result = chat.set_game_presence(&game_name, game_id, elapsed_seconds, server_address.as_deref()).await;
            if result.is_ok() {
                for community in chat.list_communities() {
                    chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Presence(
                        rekindle_types::subscription_events::PresenceEvent::CommunityMemberChanged {
                            community: community.governance_key,
                            pseudonym: community.pseudonym,
                            status: "online".to_string(),
                            game_name: Some(game_name.clone()),
                            game_id,
                        },
                    ));
                }
            }
            map_result(result)
        }
        ChatRequest::GamePresenceClear => {
            let result = chat.set_presence("online", None).await;
            if result.is_ok() {
                for community in chat.list_communities() {
                    chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Presence(
                        rekindle_types::subscription_events::PresenceEvent::CommunityMemberChanged {
                            community: community.governance_key,
                            pseudonym: community.pseudonym,
                            status: "online".to_string(),
                            game_name: None,
                            game_id: None,
                        },
                    ));
                }
            }
            map_result(result)
        }

        // ── Roles ────────────────────────────────────────────────
        ChatRequest::RoleList { community } =>
            map_result(chat.list_roles(&community).await),
        ChatRequest::RoleCreate { community, name, permissions, color, position } =>
            map_result(chat.create_role(&community, &name, permissions, color, position).await),
        ChatRequest::RoleUpdate { community, role_id, name, permissions, color } =>
            map_result(chat.update_role(&community, role_id, name.as_deref(), permissions, color).await),
        ChatRequest::RoleDelete { community, role_id } =>
            map_result(chat.delete_role(&community, role_id).await),
        ChatRequest::RoleAssign { community, member_pseudonym, role_id } =>
            map_result(chat.assign_role(&community, &member_pseudonym, role_id).await),
        ChatRequest::RoleUnassign { community, member_pseudonym, role_id } =>
            map_result(chat.unassign_role(&community, &member_pseudonym, role_id).await),

        // ── Moderation ───────────────────────────────────────────
        ChatRequest::Kick { community, target_pseudonym } =>
            map_result(chat.kick_member(&community, &target_pseudonym).await),
        ChatRequest::Ban { community, target_pseudonym, reason } =>
            map_result(chat.ban_member(&community, &target_pseudonym, reason.as_deref()).await),
        ChatRequest::Unban { community, target_pseudonym } =>
            map_result(chat.unban_member(&community, &target_pseudonym).await),
        ChatRequest::Timeout { community, target_pseudonym, duration_seconds, .. } =>
            map_result(chat.timeout_member(&community, &target_pseudonym, duration_seconds).await),
        ChatRequest::BanList { community } =>
            map_result(chat.list_bans(&community).await),

        // ── Invites ──────────────────────────────────────────────
        ChatRequest::InviteCreate { community, max_uses, expires_seconds } =>
            map_result(chat.create_invite(&community, max_uses, expires_seconds).await),
        ChatRequest::InviteList { community } =>
            map_result(chat.list_invites(&community).await),
        ChatRequest::InviteRevoke { community, invite_code } =>
            map_result(chat.revoke_invite(&community, &invite_code).await),

        // ── Social ────────────────────────────────────────────────
        ChatRequest::ReactionAdd { community, channel, message_id, emoji } =>
            map_result(chat.add_reaction(&community, &channel, &message_id, &emoji).await),
        ChatRequest::ReactionRemove { community, channel, message_id, emoji } =>
            map_result(chat.remove_reaction(&community, &channel, &message_id, &emoji).await),
        ChatRequest::PinAdd { community, channel, message_id } =>
            map_result(chat.pin_message(&community, &channel, &message_id).await),
        ChatRequest::PinRemove { community, channel, message_id } =>
            map_result(chat.unpin_message(&community, &channel, &message_id).await),
        ChatRequest::EventCreate { community, title, description, start_time, end_time, channel_id, max_attendees } =>
            map_result(chat.create_event(&community, &title, &description, start_time, end_time, channel_id.as_deref(), max_attendees).await),
        ChatRequest::EventUpdate { community, event_id, title, description, start_time, end_time, max_attendees } =>
            map_result(chat.update_event(&community, &event_id, &title, &description, start_time, end_time, max_attendees).await),
        ChatRequest::EventDelete { community, event_id } =>
            map_result(chat.delete_event(&community, &event_id).await),
        ChatRequest::EventRsvp { community, event_id, status } =>
            map_result(chat.rsvp_event(&community, &event_id, &status).await),
        ChatRequest::EventRemind { community, event_id, title, minutes_until } =>
            map_result(chat.event_reminder(&community, &event_id, &title, minutes_until).await),
        ChatRequest::ThreadCreate { community, channel, parent_message_id, title, auto_archive_seconds } =>
            map_result(chat.create_thread(&community, &channel, &parent_message_id, &title, auto_archive_seconds).await),
        ChatRequest::ThreadMessage { community, channel_id, thread_id, ciphertext, mek_generation, reply_to_id } =>
            map_result(chat.thread_message(&community, &channel_id, &thread_id, ciphertext, mek_generation, reply_to_id.as_deref()).await),
        ChatRequest::ThreadSend { community, channel, thread_id, body } => {
            let result = chat.send_thread_message(&community, &channel, &thread_id, &body).await;
            if let Ok(ref msg_id) = result {
                chat.emit_local(rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::ThreadMessagePosted {
                        community: community.clone(),
                        thread_id: thread_id.clone(),
                        message_id: msg_id.clone(),
                        sender_pseudonym: String::new(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        body: Some(body.clone()),
                    },
                ));
            }
            map_result(result)
        }
        ChatRequest::ThreadArchive { community, thread_id, archived } =>
            map_result(chat.archive_thread(&community, &thread_id, archived).await),
        ChatRequest::GameServerAdd { community, game_id, label, address } =>
            map_result(chat.add_game_server(&community, &game_id, &label, &address).await),
        ChatRequest::GameServerRemove { community, server_id } =>
            map_result(chat.remove_game_server(&community, &server_id).await),

        // ── System ───────────────────────────────────────────────
        ChatRequest::SystemAnnounce { community, body } =>
            map_result(chat.system_message(&community, &body).await),
        ChatRequest::RaidAlert { community, active } =>
            map_result(chat.raid_alert(&community, active).await),
        ChatRequest::LockdownToggle { community, locked } =>
            map_result(chat.channel_lockdown(&community, locked).await),
        ChatRequest::KickNotify { community, target_pseudonym } =>
            map_result(chat.kicked_notification(&community, &target_pseudonym).await),
        ChatRequest::BootstrapRequest { community } =>
            map_result(chat.bootstrap_request(&community).await),
        ChatRequest::BootstrapRespond { community, target_pseudonym, governance_entries, member_list, channel_meks, recent_messages, wrapped_owner_keypair } =>
            map_result(chat.bootstrap_response(&community, &target_pseudonym, governance_entries, member_list, channel_meks, recent_messages, wrapped_owner_keypair).await),
        ChatRequest::SyncRequest { community, channel_id, since_timestamp } =>
            map_result(chat.sync_request(&community, &channel_id, since_timestamp).await),
        ChatRequest::SyncRespond { community, target_pseudonym, channel_id, messages } =>
            map_result(chat.sync_response(&community, &target_pseudonym, &channel_id, messages).await),

        // ── Voice ────────────────────────────────────────────────
        ChatRequest::VoiceJoin { community, channel, muted, deafened } =>
            map_result(chat.voice_join(&community, &channel, muted, deafened).await),
        ChatRequest::VoiceLeave =>
            map_result(chat.voice_leave().await),
        ChatRequest::VoiceMute { muted } =>
            map_result(chat.voice_mute(muted).await),
        ChatRequest::VoiceDeafen { deafened } =>
            map_result(chat.voice_deafen(deafened).await),

        // ── Social list queries ─────────────────────────────────
        ChatRequest::PinList { community } =>
            map_result(chat.list_pins(&community).await),
        ChatRequest::EventList { community } =>
            map_result(chat.list_events(&community).await),
        ChatRequest::ThreadList { community } =>
            map_result(chat.list_threads(&community).await),
        ChatRequest::ReactionList { community } =>
            map_result(chat.list_reactions(&community).await),
        ChatRequest::AuditLog { community, limit } =>
            map_result(chat.list_audit_log(&community, limit).await),
        ChatRequest::ThreadHistory { thread_id, limit } =>
            map_result(chat.thread_history(&thread_id, limit)),
        ChatRequest::OnboardingConfigGet { community } =>
            map_result(chat.get_onboarding_config(&community).await),
        ChatRequest::OnboardingConfigSet { community, config } => {
            match serde_json::from_str::<rekindle_types::dht_types::OnboardingConfig>(&config) {
                Ok(cfg) => map_result(chat.set_onboarding_config(&community, &cfg).await),
                Err(e) => DaemonResponse::error(400, format!("invalid onboarding config JSON: {e}")),
            }
        }
        ChatRequest::WelcomeScreenGet { community } =>
            map_result(chat.get_welcome_screen(&community).await),
        ChatRequest::WelcomeScreenSet { community, screen } => {
            match serde_json::from_str::<rekindle_types::dht_types::WelcomeScreen>(&screen) {
                Ok(scr) => map_result(chat.set_welcome_screen(&community, &scr).await),
                Err(e) => DaemonResponse::error(400, format!("invalid welcome screen JSON: {e}")),
            }
        }
    }
}

/// Convert a ChatService Result into a DaemonResponse.
fn map_result<T: serde::Serialize>(result: Result<T, rekindle_chat::ChatError>) -> DaemonResponse {
    match result {
        Ok(val) => DaemonResponse::ok(&val),
        Err(e) => {
            tracing::warn!(error = %e, "dispatch: chat request failed");
            DaemonResponse::error(500, format!("{e}"))
        }
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
