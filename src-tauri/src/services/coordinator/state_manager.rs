//! Coordinator-side state manager: handles joins, moderation, and config changes.
//!
//! When acting as coordinator, receives `CommunityEnvelope`s from members
//! via `app_message`. Only processes Control payloads; chat/typing/presence
//! are handled by the gossip mesh and are ignored here.

use std::sync::Arc;

use rekindle_protocol::dht::community::audit_log::{
    AuditAction, AuditChange, AuditTarget,
};
use rekindle_protocol::dht::community::envelope::{
    CommunityEnvelope, ControlPayload, SignedEnvelope,
};
use rekindle_protocol::dht::community::{
    member_registry, manifest,
    permissions_v2::{calculate_permissions_v2, Permissions},
    types::MemberSummary,
};

use crate::state::AppState;
use crate::state_helpers;

use super::audit::{self, AuditLogger};
use super::automod::AutoModEnforcer;
use super::raid::RaidDetector;

/// Coordinator-side state manager: handles joins, moderation, and config changes.
pub struct StateManager {
    pub community_id: String,
    /// AutoMod enforcer for rule-based message filtering.
    automod: parking_lot::Mutex<AutoModEnforcer>,
    /// Raid detector for join flood protection.
    raid: parking_lot::Mutex<RaidDetector>,
    /// Audit logger for moderation actions (Arc for sharing with spawned tasks).
    audit_logger: Arc<parking_lot::Mutex<AuditLogger>>,
}

impl StateManager {
    /// Create a new state manager for a community.
    pub fn new(community_id: String) -> Self {
        Self {
            community_id,
            automod: parking_lot::Mutex::new(AutoModEnforcer::new(
                rekindle_protocol::dht::community::automod::AutoModConfig::default(),
            )),
            raid: parking_lot::Mutex::new(RaidDetector::new(
                rekindle_protocol::dht::community::automod::RaidProtection::default(),
            )),
            audit_logger: Arc::new(parking_lot::Mutex::new(AuditLogger::new())),
        }
    }

    /// Reload automod config (e.g. on manifest subkey 9 change).
    pub fn reload_automod(&self, config: rekindle_protocol::dht::community::automod::AutoModConfig) {
        self.automod.lock().reload_config(config);
    }

    /// Reload raid protection config.
    pub fn reload_raid_config(&self, config: rekindle_protocol::dht::community::automod::RaidProtection) {
        self.raid.lock().reload_config(config);
    }

    /// Check and auto-resolve raid status.
    pub fn check_raid_auto_resolve(&self, now_secs: u64) -> bool {
        self.raid.lock().check_auto_resolve(now_secs)
    }

    /// Set the audit record key on the logger.
    pub fn set_audit_record_key(&self, key: String) {
        self.audit_logger.lock().set_record_key(key);
    }

    /// Get a clone of the audit logger for logging actions.
    pub fn audit_logger(&self) -> Arc<parking_lot::Mutex<AuditLogger>> {
        self.audit_logger.clone()
    }
}

/// Entry point: called from veilid_service when we receive a SignedEnvelope
/// for a community where we're coordinator.
pub async fn handle_incoming_envelope(
    state: &Arc<AppState>,
    sm: &Arc<StateManager>,
    signed: SignedEnvelope,
) {
    // 1. Verify Ed25519 signature
    if let Err(e) = rekindle_protocol::dht::community::envelope::verify_envelope(&signed) {
        tracing::warn!(error = %e, "rejecting envelope with invalid signature");
        return;
    }

    // 2. Deserialize inner envelope
    let envelope: CommunityEnvelope = match serde_json::from_slice(&signed.envelope_bytes) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "rejecting envelope with invalid payload");
            return;
        }
    };

    // 3. MemberJoinRequest is special: the sender is NOT yet a member, so we
    //    must handle it before the member-existence check.
    if let CommunityEnvelope::Control(ControlPayload::MemberJoinRequest { .. }) = &envelope {
        let roles = match read_members_and_roles(state, &sm.community_id).await {
            Ok((_, r)) => r,
            Err(_) => Vec::new(),
        };
        // Build a synthetic "unknown" sender for the join handler
        let join_sender = MemberSummary {
            pseudonym_key: signed.sender_pseudonym.clone(),
            display_name: String::new(),
            role_ids: vec![0],
            joined_at: 0,
            subkey_index: 0,
            onboarding_complete: false,
            timeout_until: None,
        };
        if let CommunityEnvelope::Control(ref payload) = envelope {
            handle_control(
                state,
                sm,
                &join_sender,
                &roles,
                payload,
                &signed.sender_pseudonym,
            )
            .await;
        }
        return;
    }

    // 4. Read member info + roles for permission checking
    let (members, roles) = match read_members_and_roles(state, &sm.community_id).await {
        Ok(mr) => mr,
        Err(e) => {
            tracing::warn!(error = %e, "cannot read members/roles for permission check");
            return;
        }
    };

    let sender = members
        .iter()
        .find(|m| m.pseudonym_key == signed.sender_pseudonym);
    let Some(sender) = sender else {
        tracing::warn!(
            pseudonym = %signed.sender_pseudonym,
            "rejecting envelope from unknown member"
        );
        return;
    };

    // 5. Route by type
    match envelope {
        // Chat messages: post-hoc automod evaluation (coordinator broadcasts "hide" if needed)
        CommunityEnvelope::ChatMessage { ref channel_id, .. } => {
            // Check if sender is timed out
            if super::timeout::is_timed_out(sender, &roles, rekindle_utils::timestamp_secs()) {
                tracing::debug!(
                    pseudonym = %signed.sender_pseudonym,
                    "automod: ignoring message from timed-out member"
                );
                return;
            }

            // Run automod check
            let decision = sm.automod.lock().check_envelope(
                sender,
                &envelope,
                &roles,
                Some(channel_id.as_str()),
                None, // TODO: channel slowmode from config
                rekindle_utils::timestamp_secs(),
            );
            match decision {
                super::automod::AutoModDecision::Allow => {}
                super::automod::AutoModDecision::Block(reason) => {
                    tracing::info!(
                        pseudonym = %signed.sender_pseudonym,
                        reason = %reason,
                        "automod blocked message (post-hoc)"
                    );
                    // Broadcast a "delete message" control to all members
                    if let CommunityEnvelope::ChatMessage { ref message_id, .. } = envelope {
                        let hide_payload = ControlPayload::MessageDeleted {
                            channel_id: channel_id.clone(),
                            message_id: message_id.clone(),
                        };
                        let hide_envelope = CommunityEnvelope::Control(hide_payload);
                        broadcast_via_gossip(state, &sm.community_id, &hide_envelope);
                    }
                }
                super::automod::AutoModDecision::Timeout { duration_secs, reason } => {
                    tracing::info!(
                        pseudonym = %signed.sender_pseudonym,
                        duration = duration_secs,
                        reason = %reason,
                        "automod timed out member"
                    );
                }
                super::automod::AutoModDecision::Alert { channel_id: ch, reason } => {
                    tracing::info!(
                        pseudonym = %signed.sender_pseudonym,
                        channel = %ch,
                        reason = %reason,
                        "automod alert on message"
                    );
                }
            }
        }
        // Typing/presence handled by gossip mesh, coordinator doesn't process these
        CommunityEnvelope::TypingIndicator { .. }
        | CommunityEnvelope::PresenceUpdate { .. } => {
            tracing::trace!("coordinator ignoring typing/presence (handled by gossip)");
        }
        CommunityEnvelope::Control(ref payload) => {
            handle_control(
                state,
                sm,
                sender,
                &roles,
                payload,
                &signed.sender_pseudonym,
            )
            .await;
        }
    }
}

/// Handle a control payload from a member.
async fn handle_control(
    state: &Arc<AppState>,
    sm: &Arc<StateManager>,
    sender: &MemberSummary,
    roles: &[rekindle_protocol::dht::community::types::RoleEntryV2],
    payload: &ControlPayload,
    sender_pseudonym: &str,
) {
    let is_owner = is_community_owner(state, &sm.community_id, sender_pseudonym);
    let base_perms = calculate_permissions_v2(
        &sender.role_ids,
        roles,
        &[],
        sender_pseudonym,
        is_owner,
        sender.timeout_until,
    );

    match payload {
        // Join request - validate invite, check raid protection + onboarding, send JoinAccepted
        ControlPayload::MemberJoinRequest {
            ref display_name,
            ref route_blob,
            ref invite_code,
            ref pseudonym_key,
            ..
        } => {
            let should_reject = sm.raid.lock().should_reject_join();
            if should_reject {
                tracing::info!(
                    pseudonym = %sender_pseudonym,
                    "rejecting join: raid protection active"
                );
                return;
            }

            // Record the join for rate tracking
            let now_secs = rekindle_utils::timestamp_secs();
            let raid_actions = sm.raid.lock().record_join(now_secs);
            if let Some(actions) = raid_actions {
                tracing::warn!(
                    community = %sm.community_id,
                    actions = ?actions,
                    "raid detected — defensive actions activated"
                );
                execute_raid_actions(state, sm, &actions);
            }

            let onboard_state = state.clone();
            let onboard_community = sm.community_id.clone();
            let member_route = route_blob.clone();
            let sm_clone = Arc::clone(sm);
            let state_clone = state.clone();
            let sender_clone = sender_pseudonym.to_string();
            let display_name_clone = display_name.clone();
            let invite_code_clone = invite_code.clone();
            let joiner_pseudonym = pseudonym_key.clone();
            tokio::spawn(async move {
                // Validate invite code (if provided)
                if let Err(e) = validate_and_use_invite(
                    &state_clone,
                    &onboard_community,
                    invite_code_clone.as_deref(),
                ).await {
                    tracing::info!(
                        community = %onboard_community,
                        pseudonym = %sender_clone,
                        error = %e,
                        "rejecting join: invalid invite"
                    );
                    // Send rejection to joining member
                    if let Some(blob) = &member_route {
                        send_control_to_route(
                            &state_clone,
                            &sm_clone.community_id,
                            blob,
                            ControlPayload::JoinRejected {
                                reason: e,
                            },
                        );
                    }
                    return;
                }

                // Broadcast InviteUsed if an invite was consumed
                if let Some(ref code) = invite_code_clone {
                    notify_invite_used(&state_clone, &sm_clone, code).await;
                }

                // Add member to registry
                if let Err(e) = add_member_to_registry(
                    &state_clone,
                    &onboard_community,
                    &joiner_pseudonym,
                    &display_name_clone,
                ).await {
                    tracing::warn!(
                        community = %onboard_community,
                        error = %e,
                        "failed to add member to registry"
                    );
                }

                // Send JoinAccepted to the joining member
                if let Some(blob) = &member_route {
                    send_join_accepted(
                        &state_clone,
                        &onboard_community,
                        blob,
                    ).await;
                } else {
                    tracing::error!(
                        community = %onboard_community,
                        pseudonym = %joiner_pseudonym,
                        "MemberJoinRequest has no route_blob — cannot send JoinAccepted"
                    );
                }

                // Broadcast MemberJoined to existing members via gossip mesh.
                let joined_payload = ControlPayload::MemberJoined {
                    pseudonym_key: joiner_pseudonym.clone(),
                    display_name: display_name_clone.clone(),
                    role_ids: vec![0, 1],
                };
                let joined_envelope = CommunityEnvelope::Control(joined_payload);
                broadcast_via_gossip(&state_clone, &sm_clone.community_id, &joined_envelope);

                // Emit MemberJoined locally so the coordinator's own frontend sees
                // the new member.
                emit_local_member_joined(
                    &state_clone,
                    &onboard_community,
                    &joiner_pseudonym,
                    &display_name_clone,
                );

                // Check onboarding and send questions to new member if needed
                match super::onboarding::check_onboarding(
                    &onboard_state,
                    &onboard_community,
                )
                .await
                {
                    Ok(Some(questions_payload)) => {
                        tracing::debug!(
                            community = %onboard_community,
                            pseudonym = %sender_clone,
                            "sending onboarding questions to new member"
                        );
                        if let Some(blob) = &member_route {
                            send_control_to_route(
                                &state_clone,
                                &sm_clone.community_id,
                                blob,
                                questions_payload,
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(
                            community = %onboard_community,
                            error = %e,
                            "failed to check onboarding, admitting anyway"
                        );
                    }
                }

                broadcast_system_message(
                    &state_clone,
                    &sm_clone,
                    &format!("{display_name_clone} joined the community"),
                );
            });
        }

        // Member leave — broadcast via gossip + system message
        ControlPayload::MemberLeave { ref pseudonym_key } => {
            let who = pseudonym_key.clone();
            let leave_envelope = CommunityEnvelope::Control(payload.clone());
            broadcast_via_gossip(state, &sm.community_id, &leave_envelope);
            broadcast_system_message(state, sm, &format!("{who} left the community"));
        }

        // Moderation - requires KICK_MEMBERS
        ControlPayload::Kick { target_pseudonym, .. } => {
            if base_perms.has(Permissions::KICK_MEMBERS) {
                let target = target_pseudonym.clone();
                let kick_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &kick_envelope);
                broadcast_system_message(state, sm, &format!("{target} was kicked"));
                log_audit(state, sm, AuditAction::MemberKick, AuditTarget::Member(target), vec![], None);
            }
        }
        ControlPayload::Ban { target_pseudonym, .. } => {
            if base_perms.has(Permissions::BAN_MEMBERS) {
                let target = target_pseudonym.clone();
                let ban_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &ban_envelope);
                broadcast_system_message(state, sm, &format!("{target} was banned"));
                log_audit(state, sm, AuditAction::MemberBan, AuditTarget::Member(target), vec![], None);
            }
        }
        ControlPayload::Unban { target_pseudonym, .. } => {
            if base_perms.has(Permissions::BAN_MEMBERS) {
                let target = target_pseudonym.clone();
                let unban_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &unban_envelope);
                log_audit(state, sm, AuditAction::MemberUnban, AuditTarget::Member(target), vec![], None);
            }
        }
        ControlPayload::TimeoutMember { target_pseudonym, duration_seconds, .. } => {
            if base_perms.has(Permissions::MODERATE_MEMBERS) {
                let target = target_pseudonym.clone();
                let timeout_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &timeout_envelope);
                log_audit(state, sm, AuditAction::MemberTimeout, AuditTarget::Member(target), vec![
                    AuditChange { field: "duration_seconds".into(), old_value: None, new_value: Some(duration_seconds.to_string()) },
                ], None);
            }
        }
        ControlPayload::RemoveTimeout { target_pseudonym, .. } => {
            if base_perms.has(Permissions::MODERATE_MEMBERS) {
                let target = target_pseudonym.clone();
                let rm_timeout_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &rm_timeout_envelope);
                log_audit(state, sm, AuditAction::MemberTimeoutRemove, AuditTarget::Member(target), vec![], None);
            }
        }

        // Channel management - requires MANAGE_CHANNELS
        ControlPayload::CreateChannel { name, .. } => {
            if base_perms.has(Permissions::MANAGE_CHANNELS) {
                let ch_name = name.clone();
                let create_ch_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &create_ch_envelope);
                log_audit(state, sm, AuditAction::ChannelCreate, AuditTarget::Community, vec![
                    AuditChange { field: "name".into(), old_value: None, new_value: Some(ch_name) },
                ], None);
            }
        }
        ControlPayload::DeleteChannel { channel_id, .. } => {
            if base_perms.has(Permissions::MANAGE_CHANNELS) {
                let ch_id = channel_id.clone();
                let del_ch_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &del_ch_envelope);
                log_audit(state, sm, AuditAction::ChannelDelete, AuditTarget::Channel(ch_id), vec![], None);
            }
        }
        ControlPayload::RenameChannel { .. }
        | ControlPayload::SetChannelTopic { .. }
        | ControlPayload::ReorderChannels { .. }
        | ControlPayload::SetSlowmode { .. }
        | ControlPayload::MoveChannel { .. }
        | ControlPayload::CreateCategory { .. }
        | ControlPayload::DeleteCategory { .. }
        | ControlPayload::RenameCategory { .. }
        | ControlPayload::ReorderCategories { .. } => {
            if base_perms.has(Permissions::MANAGE_CHANNELS) {
                let ch_mgmt_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &ch_mgmt_envelope);
            }
        }

        // Role management + channel overwrites - requires MANAGE_ROLES
        ControlPayload::CreateRole { name, .. } => {
            if base_perms.has(Permissions::MANAGE_ROLES) {
                let role_name = name.clone();
                let create_role_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &create_role_envelope);
                log_audit(state, sm, AuditAction::RoleCreate, AuditTarget::Community, vec![
                    AuditChange { field: "name".into(), old_value: None, new_value: Some(role_name) },
                ], None);
            }
        }
        ControlPayload::DeleteRole { role_id, .. } => {
            if base_perms.has(Permissions::MANAGE_ROLES) {
                let rid = *role_id;
                let del_role_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &del_role_envelope);
                log_audit(state, sm, AuditAction::RoleDelete, AuditTarget::Role(rid), vec![], None);
            }
        }
        ControlPayload::AssignRole { target_pseudonym, role_id, .. }
        | ControlPayload::UnassignRole { target_pseudonym, role_id, .. } => {
            if base_perms.has(Permissions::MANAGE_ROLES) {
                let target = target_pseudonym.clone();
                let rid = *role_id;
                let role_assign_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &role_assign_envelope);
                log_audit(state, sm, AuditAction::MemberRoleUpdate, AuditTarget::Member(target), vec![
                    AuditChange { field: "role_id".into(), old_value: None, new_value: Some(rid.to_string()) },
                ], None);
            }
        }
        ControlPayload::EditRole { .. }
        | ControlPayload::SetChannelOverwrite { .. }
        | ControlPayload::DeleteChannelOverwrite { .. } => {
            if base_perms.has(Permissions::MANAGE_ROLES) {
                let role_edit_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &role_edit_envelope);
            }
        }

        // All other control payloads — permission checks + broadcast via gossip
        other => handle_control_extended(state, sm, base_perms, other, sender_pseudonym).await,
    }
}

/// Extended control payload handling (split from `handle_control` for clippy line limit).
async fn handle_control_extended(
    state: &Arc<AppState>,
    sm: &Arc<StateManager>,
    base_perms: Permissions,
    payload: &ControlPayload,
    sender_pseudonym: &str,
) {
    match payload {
        // Community metadata, game servers - requires MANAGE_COMMUNITY
        ControlPayload::UpdateCommunity { .. }
        | ControlPayload::ListInvites
        | ControlPayload::AddGameServer { .. }
        | ControlPayload::RemoveGameServer { .. } => {
            if base_perms.has(Permissions::MANAGE_COMMUNITY) {
                let mgmt_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &mgmt_envelope);
            }
        }

        // Create invite - requires CREATE_INSTANT_INVITE
        // Awaited (not spawned) so the invite is persisted to the DHT manifest
        // before any client can list invites and get a stale empty result.
        ControlPayload::CreateInvite { code, max_uses, expires_in_seconds } => {
            if base_perms.has(Permissions::CREATE_INSTANT_INVITE) {
                let creator = sender_pseudonym.to_string();
                match create_invite_entry(
                    state,
                    &sm.community_id,
                    &creator,
                    code,
                    *max_uses,
                    *expires_in_seconds,
                ).await {
                    Ok(entry) => {
                        // Broadcast InviteCreated to all members via gossip
                        let broadcast = ControlPayload::InviteCreated {
                            code: entry.code.clone(),
                            created_by: entry.created_by.clone(),
                            max_uses: if entry.max_uses > 0 { Some(entry.max_uses) } else { None },
                            uses: entry.use_count,
                            expires_at: entry.expires_at,
                            created_at: entry.created_at,
                        };
                        let envelope = CommunityEnvelope::Control(broadcast);
                        broadcast_via_gossip(state, &sm.community_id, &envelope);
                        tracing::info!(
                            community = %sm.community_id,
                            code = %entry.code,
                            "invite created and persisted to manifest"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            community = %sm.community_id,
                            error = %e,
                            "failed to create invite"
                        );
                    }
                }
            }
        }

        // Revoke invite - requires MANAGE_COMMUNITY
        // Awaited (not spawned) so the revocation is persisted before clients can list invites.
        ControlPayload::RevokeInvite { code } => {
            if base_perms.has(Permissions::MANAGE_COMMUNITY) {
                match revoke_invite_entry(state, &sm.community_id, code).await {
                    Ok(()) => {
                        // Broadcast InviteRevoked to all members via gossip
                        let broadcast = ControlPayload::InviteRevoked { code: code.clone() };
                        let envelope = CommunityEnvelope::Control(broadcast);
                        broadcast_via_gossip(state, &sm.community_id, &envelope);
                        tracing::info!(
                            community = %sm.community_id,
                            code = %code,
                            "invite revoked"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            community = %sm.community_id,
                            code = %code,
                            error = %e,
                            "failed to revoke invite"
                        );
                    }
                }
            }
        }

        // Events management - requires MANAGE_EVENTS
        ControlPayload::CreateEvent { .. }
        | ControlPayload::EditEvent { .. }
        | ControlPayload::DeleteEvent { .. }
        | ControlPayload::CancelEvent { .. } => {
            if base_perms.has(Permissions::MANAGE_EVENTS) {
                let event_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &event_envelope);
            }
        }

        // Reactions
        ControlPayload::AddReaction { .. } | ControlPayload::RemoveReaction { .. } => {
            if base_perms.has(Permissions::ADD_REACTIONS) {
                let reaction_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &reaction_envelope);
            }
        }

        // Message management (pin/unpin/delete) - requires MANAGE_MESSAGES
        ControlPayload::PinMessage { .. }
        | ControlPayload::UnpinMessage { .. }
        | ControlPayload::DeleteMessage { .. } => {
            if base_perms.has(Permissions::MANAGE_MESSAGES) {
                let msg_mgmt_envelope = CommunityEnvelope::Control(payload.clone());
                broadcast_via_gossip(state, &sm.community_id, &msg_mgmt_envelope);
            }
        }

        // Onboarding answers — process and assign roles
        ControlPayload::SubmitOnboardingAnswers { answers } => {
            let onboard_state = state.clone();
            let onboard_community = sm.community_id.clone();
            let onboard_answers = answers.clone();
            let onboard_pseudonym = sender_pseudonym.to_string();
            let state_clone = state.clone();
            tokio::spawn(async move {
                match super::onboarding::process_answers(
                    &onboard_state,
                    &onboard_community,
                    &onboard_answers,
                )
                .await
                {
                    Ok(role_ids) => {
                        if !role_ids.is_empty() {
                            if let Err(e) = assign_onboarding_roles(
                                &state_clone,
                                &onboard_community,
                                &onboard_pseudonym,
                                &role_ids,
                            )
                            .await
                            {
                                tracing::warn!(
                                    community = %onboard_community,
                                    pseudonym = %onboard_pseudonym,
                                    error = %e,
                                    "failed to assign onboarding roles"
                                );
                            } else {
                                tracing::info!(
                                    community = %onboard_community,
                                    pseudonym = %onboard_pseudonym,
                                    roles = ?role_ids,
                                    "onboarding roles assigned"
                                );
                                let notification = ControlPayload::MemberRolesChanged {
                                    pseudonym_key: onboard_pseudonym.clone(),
                                    role_ids: role_ids.clone(),
                                };
                                let envelope = CommunityEnvelope::Control(notification);
                                broadcast_via_gossip(&state_clone, &onboard_community, &envelope);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            community = %onboard_community,
                            pseudonym = %onboard_pseudonym,
                            error = %e,
                            "failed to process onboarding answers"
                        );
                    }
                }
            });
        }

        // Read-only operations & broadcast variants - broadcast via gossip
        _ => {
            let other_envelope = CommunityEnvelope::Control(payload.clone());
            broadcast_via_gossip(state, &sm.community_id, &other_envelope);
        }
    }
}

/// Send a control payload directly to a specific route blob.
///
/// Used for point-to-point messages to joiners (JoinRejected, OnboardingQuestions)
/// that cannot go through the gossip mesh because the recipient is not yet a member.
fn send_control_to_route(
    state: &Arc<AppState>,
    community_id: &str,
    target_route_blob: &[u8],
    payload: ControlPayload,
) {
    let Some(rc) = state_helpers::routing_context(state) else {
        tracing::debug!("no routing context for send_control_to_route");
        return;
    };

    // Sign envelope as coordinator so the receiver can parse it as SignedEnvelope.
    let (my_pseudonym, signing_key) = {
        let communities = state.communities.read();
        let Some(c) = communities.get(community_id) else {
            return;
        };
        let pseudonym = c.my_pseudonym_key.clone().unwrap_or_default();
        let secret = state.identity_secret.lock();
        let key = match *secret {
            Some(ref s) => {
                rekindle_crypto::group::pseudonym::derive_community_pseudonym(s, community_id)
            }
            None => return,
        };
        (pseudonym, key)
    };

    let envelope = CommunityEnvelope::Control(payload);
    let envelope_bytes = match serde_json::to_vec(&envelope) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize control payload");
            return;
        }
    };

    let signed = rekindle_protocol::dht::community::envelope::sign_envelope(
        &signing_key,
        community_id,
        &my_pseudonym,
        &envelope_bytes,
    );
    let signed_bytes = match serde_json::to_vec(&signed) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize signed envelope");
            return;
        }
    };

    let blob = target_route_blob.to_vec();
    let cid = community_id.to_string();
    tokio::spawn(async move {
        match rc.api().import_remote_private_route(blob) {
            Ok(route_id) => {
                if let Err(e) = rc
                    .app_message(veilid_core::Target::RouteId(route_id), signed_bytes)
                    .await
                {
                    tracing::warn!(
                        community = %cid,
                        error = %e,
                        "send_control_to_route delivery failed"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    community = %cid,
                    error = %e,
                    "failed to import route for send_control_to_route"
                );
            }
        }
    });
}

/// Broadcast a coordinator-originated control envelope via the gossip mesh.
///
/// Signs the envelope with the coordinator's pseudonym key, then sends
/// to all gossip peers via `gossip_send_raw()`.
pub(crate) fn broadcast_via_gossip(
    state: &Arc<AppState>,
    community_id: &str,
    envelope: &CommunityEnvelope,
) {
    use rekindle_protocol::dht::community::envelope::sign_envelope;

    let my_pseudonym = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else { return };
        cs.my_pseudonym_key.clone().unwrap_or_default()
    };

    let Some(secret) = state.identity_secret.lock().as_ref().copied() else { return };
    let signing_key = rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
    let envelope_bytes = match serde_json::to_vec(&envelope) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize envelope for gossip broadcast");
            return;
        }
    };

    let signed = sign_envelope(&signing_key, community_id, &my_pseudonym, &envelope_bytes);
    let signed_bytes = match serde_json::to_vec(&signed) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize signed envelope for gossip broadcast");
            return;
        }
    };

    // Insert into our dedup cache so we don't re-process our own broadcast
    {
        let dedup_key = extract_broadcast_dedup_key(envelope);
        state.dedup_cache.lock().check_and_insert(community_id, &my_pseudonym, &dedup_key);
    }

    gossip_send_raw(state, community_id, &signed_bytes);
}

/// Extract a dedup key from a coordinator broadcast envelope.
fn extract_broadcast_dedup_key(envelope: &CommunityEnvelope) -> String {
    use blake2::{Blake2b, Digest, digest::consts::U16};
    let bytes = serde_json::to_vec(envelope).unwrap_or_default();
    let mut h = Blake2b::<U16>::new();
    h.update(&bytes);
    hex::encode(h.finalize())
}

/// Low-level: send raw signed bytes to all gossip peers for a community.
fn gossip_send_raw(state: &Arc<AppState>, community_id: &str, signed_bytes: &[u8]) {
    let Some(rc) = crate::state_helpers::routing_context(state) else { return };

    let peers: Vec<Vec<u8>> = {
        let communities = state.communities.read();
        let Some(cs) = communities.get(community_id) else { return };
        let Some(ref gossip) = cs.gossip else { return };
        gossip.peers.values().cloned().collect()
    };

    for route_blob in peers {
        let rc = rc.clone();
        let data = signed_bytes.to_vec();
        tokio::spawn(async move {
            match rc.api().import_remote_private_route(route_blob) {
                Ok(route_id) => {
                    let _ = rc.app_message(veilid_core::Target::RouteId(route_id), data).await;
                }
                Err(e) => tracing::trace!(error = %e, "gossip broadcast: route import failed"),
            }
        });
    }
}

/// Emit `CommunityEvent::MemberJoined` locally on the coordinator's frontend.
///
/// The coordinator processes messages locally, so broadcast_via_gossip may not
/// reach the coordinator's own UI. This function fills that gap by persisting
/// the new member to SQLite and emitting the event directly via the Tauri app handle.
fn emit_local_member_joined(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
) {
    use tauri::{Emitter, Manager};

    let Some(app_handle) = state_helpers::app_handle(state) else {
        tracing::debug!("no app handle for emit_local_member_joined");
        return;
    };

    // Persist to SQLite so get_community_members includes the new member
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    let dn = display_name.to_string();
    let role_ids = vec![0_u32, 1];
    let rids = role_ids.clone();
    crate::db_helpers::db_fire(
        pool.inner(),
        "persist MemberJoined (coordinator local)",
        move |conn| {
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
        },
    );

    let _ = app_handle.emit(
        "community-event",
        crate::channels::CommunityEvent::MemberJoined {
            community_id: community_id.to_string(),
            pseudonym_key: pseudonym_key.to_string(),
            display_name: display_name.to_string(),
            role_ids,
        },
    );
}

/// Read members and roles for permission checking.
async fn read_members_and_roles(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<
    (
        Vec<MemberSummary>,
        Vec<rekindle_protocol::dht::community::types::RoleEntryV2>,
    ),
    String,
> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let (manifest_key, registry_key) = {
        let communities = state.communities.read();
        let c = communities
            .get(community_id)
            .ok_or("community not found")?;
        (
            c.manifest_key
                .clone()
                .or_else(|| Some(c.id.clone()))
                .ok_or("no manifest key")?,
            c.member_registry_key.clone(),
        )
    };

    let members = if let Some(ref reg_key) = registry_key {
        member_registry::read_member_index(&mgr, reg_key)
            .await
            .map_err(|e| format!("read member index: {e}"))?
    } else {
        Vec::new()
    };

    let roles = manifest::read_roles(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read roles: {e}"))?;

    Ok((members, roles))
}

/// Broadcast a system message to all members via gossip (join/leave/kick/ban events).
fn broadcast_system_message(
    state: &Arc<AppState>,
    sm: &Arc<StateManager>,
    body: &str,
) {
    let payload = ControlPayload::SystemMessage {
        body: body.to_string(),
        timestamp: rekindle_utils::timestamp_secs(),
    };
    let envelope = CommunityEnvelope::Control(payload);
    broadcast_via_gossip(state, &sm.community_id, &envelope);
}

/// Execute raid defense actions triggered by the raid detector.
fn execute_raid_actions(
    state: &Arc<AppState>,
    sm: &Arc<StateManager>,
    actions: &[rekindle_protocol::dht::community::automod::RaidAction],
) {
    use rekindle_protocol::dht::community::automod::RaidAction;

    for action in actions {
        match action {
            RaidAction::PauseInvites | RaidAction::RestrictNewMembers => {
                // Handled by RaidDetector state flags — checked in should_reject_join()
            }
            RaidAction::AlertOwners => {
                // Broadcast a raid alert to the community via gossip
                let alert_payload =
                    ControlPayload::RaidAlert { active: true };
                let envelope = CommunityEnvelope::Control(alert_payload);
                let state = state.clone();
                let community_id = sm.community_id.clone();
                let sm_clone = Arc::clone(sm);
                tokio::spawn(async move {
                    broadcast_via_gossip(&state, &community_id, &envelope);
                    log_audit(
                        &state,
                        &sm_clone,
                        AuditAction::AutoModActionExecuted,
                        AuditTarget::Community,
                        vec![AuditChange {
                            field: "raid_action".into(),
                            old_value: None,
                            new_value: Some("alert_owners".into()),
                        }],
                        Some("raid detected".into()),
                    );
                });
            }
            RaidAction::LockdownChannels => {
                // Broadcast a lockdown notification to all members via gossip.
                // Members restrict SEND_MESSAGES client-side for non-admins.
                let lockdown_payload =
                    ControlPayload::ChannelLockdown { locked: true };
                let envelope = CommunityEnvelope::Control(lockdown_payload);
                let state = state.clone();
                let community_id = sm.community_id.clone();
                let sm_clone = Arc::clone(sm);
                tokio::spawn(async move {
                    broadcast_via_gossip(&state, &community_id, &envelope);
                    log_audit(
                        &state,
                        &sm_clone,
                        AuditAction::AutoModActionExecuted,
                        AuditTarget::Community,
                        vec![AuditChange {
                            field: "raid_action".into(),
                            old_value: None,
                            new_value: Some("lockdown_channels".into()),
                        }],
                        Some("raid detected".into()),
                    );
                });
            }
        }
    }
}

/// Fire-and-forget audit log entry (spawns async task).
fn log_audit(
    state: &Arc<AppState>,
    sm: &Arc<StateManager>,
    action: AuditAction,
    target: AuditTarget,
    changes: Vec<AuditChange>,
    reason: Option<String>,
) {
    let state = state.clone();
    let community_id = sm.community_id.clone();
    let logger = sm.audit_logger.clone();
    tokio::spawn(async move {
        audit::log_action(&state, &community_id, &logger, action, target, changes, reason).await;
    });
}

/// Assign additional roles to a member after onboarding answer processing.
///
/// Reads the current member index, adds the new role IDs to the member's existing roles,
/// and writes the updated index back to the member registry.
async fn assign_onboarding_roles(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    new_role_ids: &[u32],
) -> Result<(), String> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let registry_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.member_registry_key
            .clone()
            .ok_or("no member registry key")?
    };

    let mut members = member_registry::read_member_index(&mgr, &registry_key)
        .await
        .map_err(|e| format!("read member index: {e}"))?;

    if let Some(member) = members.iter_mut().find(|m| m.pseudonym_key == pseudonym_key) {
        for &rid in new_role_ids {
            if !member.role_ids.contains(&rid) {
                member.role_ids.push(rid);
            }
        }
        member_registry::write_member_index(&mgr, &registry_key, &members)
            .await
            .map_err(|e| format!("write member index: {e}"))?;

        Ok(())
    } else {
        Err(format!("member {pseudonym_key} not found in registry"))
    }
}

/// Add a new member to the member registry index.
async fn add_member_to_registry(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
) -> Result<(), String> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let registry_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.member_registry_key
            .clone()
            .ok_or("no member registry key")?
    };

    let mut members = member_registry::read_member_index(&mgr, &registry_key)
        .await
        .map_err(|e| format!("read member index: {e}"))?;

    // Don't add duplicates
    if members.iter().any(|m| m.pseudonym_key == pseudonym_key) {
        return Ok(());
    }

    let now = rekindle_utils::timestamp_secs();
    let next_subkey = members.iter().map(|m| m.subkey_index).max().unwrap_or(0) + 1;

    members.push(MemberSummary {
        pseudonym_key: pseudonym_key.to_string(),
        display_name: display_name.to_string(),
        role_ids: vec![0, 1], // @everyone + member
        timeout_until: None,
        joined_at: now,
        subkey_index: next_subkey,
        onboarding_complete: false,
    });

    member_registry::write_member_index(&mgr, &registry_key, &members)
        .await
        .map_err(|e| format!("write member index: {e}"))?;

    tracing::debug!(
        community = %community_id,
        pseudonym = %pseudonym_key,
        "added member to registry"
    );

    Ok(())
}

/// Send a JoinAccepted envelope to a newly joined member with community data.
///
/// Retries up to 3 times because JoinAccepted is critical — without it the
/// joiner has no MEK and cannot participate.
async fn send_join_accepted(
    state: &Arc<AppState>,
    community_id: &str,
    target_route_blob: &[u8],
) {
    let Some(rc) = state_helpers::routing_context(state) else {
        return;
    };
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let manifest_key = {
        let communities = state.communities.read();
        match communities.get(community_id) {
            Some(c) => c.manifest_key.clone().unwrap_or_else(|| c.id.clone()),
            None => return,
        }
    };

    // Read community data from manifest
    let channels = manifest::read_channels(&mgr, &manifest_key)
        .await
        .unwrap_or_default();
    let categories = manifest::read_categories(&mgr, &manifest_key)
        .await
        .unwrap_or_default();
    let roles = manifest::read_roles(&mgr, &manifest_key)
        .await
        .unwrap_or_default();

    // Read member list from registry
    let registry_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.member_registry_key.clone())
    };
    let members = if let Some(ref rk) = registry_key {
        member_registry::read_member_index(&mgr, rk)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Get MEK for this community
    let mek_generation = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map_or(0, |c| c.mek_generation)
    };

    // Get raw MEK wire bytes (generation + key) from cache to send to joiner
    let mek_bytes = {
        let cache = state.mek_cache.lock();
        cache
            .get(community_id)
            .map(rekindle_crypto::group::media_key::MediaEncryptionKey::to_wire_bytes)
    };

    let payload = ControlPayload::JoinAccepted {
        mek_encrypted: mek_bytes.unwrap_or_default(),
        mek_generation,
        channels: channels
            .iter()
            .filter_map(|c| serde_json::to_value(c).ok())
            .collect(),
        categories: categories
            .iter()
            .filter_map(|c| serde_json::to_value(c).ok())
            .collect(),
        role_ids: vec![0, 1], // Default roles for new member
        roles: roles
            .iter()
            .filter_map(|r| serde_json::to_value(r).ok())
            .collect(),
        members: members
            .iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect(),
    };

    // JoinAccepted is critical — the joiner cannot participate without MEK.
    // Sign and send directly with retry.
    let signed_bytes = {
        let (my_pseudonym, signing_key) = {
            let communities = state.communities.read();
            let Some(c) = communities.get(community_id) else { return };
            let pseudonym = c.my_pseudonym_key.clone().unwrap_or_default();
            let secret = state.identity_secret.lock();
            let key = match *secret {
                Some(ref s) => {
                    rekindle_crypto::group::pseudonym::derive_community_pseudonym(s, community_id)
                }
                None => return,
            };
            (pseudonym, key)
        };

        let envelope = CommunityEnvelope::Control(payload);
        let envelope_bytes = match serde_json::to_vec(&envelope) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, community = %community_id, "failed to serialize JoinAccepted");
                return;
            }
        };
        let signed = rekindle_protocol::dht::community::envelope::sign_envelope(
            &signing_key,
            community_id,
            &my_pseudonym,
            &envelope_bytes,
        );
        match serde_json::to_vec(&signed) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, community = %community_id, "failed to serialize signed JoinAccepted");
                return;
            }
        }
    };

    let Some(rc) = state_helpers::routing_context(state) else {
        tracing::warn!("no routing context for JoinAccepted delivery");
        return;
    };

    let blob = target_route_blob.to_vec();
    let cid = community_id.to_string();
    for attempt in 0..3 {
        match rc.api().import_remote_private_route(blob.clone()) {
            Ok(route_id) => {
                match rc
                    .app_message(
                        veilid_core::Target::RouteId(route_id),
                        signed_bytes.clone(),
                    )
                    .await
                {
                    Ok(()) => {
                        tracing::info!(
                            community = %cid,
                            attempt,
                            "JoinAccepted delivered successfully"
                        );
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(
                            community = %cid,
                            attempt,
                            error = %e,
                            "JoinAccepted delivery failed"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    community = %cid,
                    attempt,
                    error = %e,
                    "failed to import joiner route for JoinAccepted"
                );
            }
        }
        // Brief delay before retry
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    tracing::error!(
        community = %cid,
        "JoinAccepted delivery failed after 3 attempts — joiner will not receive MEK"
    );
}

/// Notify all members that an invite was used (for frontend tracking).
async fn notify_invite_used(
    state: &Arc<AppState>,
    sm: &Arc<StateManager>,
    code: &str,
) {
    // Read the updated invite to get the new use count
    let Some(rc) = state_helpers::routing_context(state) else {
        return;
    };
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let manifest_key = {
        let communities = state.communities.read();
        match communities.get(&sm.community_id) {
            Some(c) => c.manifest_key.clone().unwrap_or_else(|| c.id.clone()),
            None => return,
        }
    };

    let invites = manifest::read_invites(&mgr, &manifest_key)
        .await
        .unwrap_or_default();

    if let Some(inv) = invites.iter().find(|i| i.code == code) {
        let broadcast = ControlPayload::InviteUsed {
            code: code.to_string(),
            new_use_count: inv.use_count,
        };
        let envelope = CommunityEnvelope::Control(broadcast);
        broadcast_via_gossip(state, &sm.community_id, &envelope);
    }
}

/// Check if a pseudonym is the community owner.
fn is_community_owner(state: &Arc<AppState>, community_id: &str, pseudonym: &str) -> bool {
    let communities = state.communities.read();
    if let Some(cs) = communities.get(community_id) {
        // The owner is the one with the owner role (id=4) in their role_ids
        if let Some(my_key) = &cs.my_pseudonym_key {
            if my_key == pseudonym {
                return cs.my_role_ids.contains(&4);
            }
        }
    }
    false
}

/// Create an invite entry, persist to manifest, return the entry.
async fn create_invite_entry(
    state: &Arc<AppState>,
    community_id: &str,
    created_by: &str,
    code: &str,
    max_uses: Option<u32>,
    expires_in_seconds: Option<u64>,
) -> Result<rekindle_protocol::dht::community::types::InviteEntry, String> {
    use rekindle_protocol::dht::community::types::InviteEntry;

    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let manifest_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.manifest_key.clone().unwrap_or_else(|| c.id.clone())
    };

    let now = rekindle_utils::timestamp_secs();
    let expires_at = expires_in_seconds.map(|s| now + s);

    let entry = InviteEntry {
        code: code.to_string(),
        created_by: created_by.to_string(),
        created_at: now,
        expires_at,
        max_uses: max_uses.unwrap_or(0),
        use_count: 0,
    };

    // Read existing invites, append new one, write back
    let mut invites = manifest::read_invites(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read invites: {e}"))?;

    // Prune expired invites while we're at it
    invites.retain(|inv| {
        inv.expires_at.is_none_or(|exp| exp > now)
    });

    invites.push(entry.clone());

    manifest::write_invites(&mgr, &manifest_key, &invites)
        .await
        .map_err(|e| format!("write invites: {e}"))?;

    Ok(entry)
}

/// Revoke an invite by code: remove from manifest.
async fn revoke_invite_entry(
    state: &Arc<AppState>,
    community_id: &str,
    code: &str,
) -> Result<(), String> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let manifest_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.manifest_key.clone().unwrap_or_else(|| c.id.clone())
    };

    let mut invites = manifest::read_invites(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read invites: {e}"))?;

    let original_len = invites.len();
    invites.retain(|inv| inv.code != code);

    if invites.len() == original_len {
        return Err(format!("invite code {code} not found"));
    }

    manifest::write_invites(&mgr, &manifest_key, &invites)
        .await
        .map_err(|e| format!("write invites: {e}"))?;

    Ok(())
}

/// Validate an invite code: check it exists, is not expired, has not exhausted uses.
/// On success, increments use_count and writes back to manifest.
/// Returns Ok(()) on valid invite, or Ok(()) if no code given (open community).
pub(crate) async fn validate_and_use_invite(
    state: &Arc<AppState>,
    community_id: &str,
    invite_code: Option<&str>,
) -> Result<(), String> {
    let Some(code) = invite_code else {
        // No code required — check if community has any active invites
        // For now, allow open joins (no invite required)
        return Ok(());
    };

    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let manifest_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.manifest_key.clone().unwrap_or_else(|| c.id.clone())
    };

    let mut invites = manifest::read_invites(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read invites: {e}"))?;

    let now = rekindle_utils::timestamp_secs();
    let invite = invites
        .iter_mut()
        .find(|inv| inv.code == code)
        .ok_or_else(|| format!("invalid invite code: {code}"))?;

    // Check expiry
    if let Some(expires_at) = invite.expires_at {
        if now > expires_at {
            return Err("invite has expired".into());
        }
    }

    // Check max uses
    if invite.max_uses > 0 && invite.use_count >= invite.max_uses {
        return Err("invite has reached maximum uses".into());
    }

    // Increment use count
    invite.use_count += 1;
    let new_count = invite.use_count;
    let invite_code_owned = invite.code.clone();

    manifest::write_invites(&mgr, &manifest_key, &invites)
        .await
        .map_err(|e| format!("write invites: {e}"))?;

    tracing::debug!(
        community = %community_id,
        code = %invite_code_owned,
        use_count = new_count,
        "invite used"
    );

    Ok(())
}
