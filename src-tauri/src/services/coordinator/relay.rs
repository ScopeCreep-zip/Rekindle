//! Coordinator-side message relay: receives envelopes and fans out to online members.
//!
//! When acting as coordinator, receives `CommunityEnvelope`s from members
//! via `app_message` and relays to all online members. Validates permissions,
//! enforces timeouts, and can persist messages to DHT.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::Semaphore;

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
use super::automod::{AutoModDecision, AutoModEnforcer};
use super::raid::RaidDetector;
use super::timeout;

/// Maximum concurrent fan-out sends to avoid overwhelming Veilid.
const MAX_CONCURRENT_FANOUT: usize = 16;

/// Coordinator-side relay: receives envelopes and fans out to online members.
pub struct RelayService {
    pub community_id: String,
    /// Cached online members: pseudonym_key -> route_blob.
    pub online_members: RwLock<HashMap<String, Vec<u8>>>,
    /// Bound concurrent fan-out sends.
    fan_out_semaphore: Arc<Semaphore>,
    /// AutoMod enforcer for rule-based message filtering.
    automod: parking_lot::Mutex<AutoModEnforcer>,
    /// Raid detector for join flood protection.
    raid: parking_lot::Mutex<RaidDetector>,
    /// Audit logger for moderation actions (Arc for sharing with spawned tasks).
    audit_logger: Arc<parking_lot::Mutex<AuditLogger>>,
}

impl RelayService {
    /// Create a new relay service for a community.
    pub fn new(community_id: String) -> Self {
        Self {
            community_id,
            online_members: RwLock::new(HashMap::new()),
            fan_out_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_FANOUT)),
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

    /// Update the online member list from the member registry.
    pub(crate) async fn refresh_online_members(&self, state: &Arc<AppState>) {
        let Some(rc) = state_helpers::routing_context(state) else {
            return;
        };

        let registry_key = {
            let communities = state.communities.read();
            communities
                .get(&self.community_id)
                .and_then(|c| c.member_registry_key.clone())
        };

        let Some(registry_key) = registry_key else {
            return;
        };

        let mgr = rekindle_protocol::dht::DHTManager::new(rc);

        // Read member index to know how many members there are
        let members = match member_registry::read_member_index(&mgr, &registry_key).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read member index for relay");
                return;
            }
        };

        // Read each member's presence to get their route blob
        let mut online = HashMap::new();
        for member in &members {
            match member_registry::read_member_presence(
                &mgr,
                &registry_key,
                member.subkey_index,
            )
            .await
            {
                Ok(Some(presence)) => {
                    if presence.status != "offline" {
                        if let Some(route_blob) = presence.route_blob {
                            online.insert(member.pseudonym_key.clone(), route_blob);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::trace!(
                        member = %member.pseudonym_key,
                        error = %e,
                        "failed to read member presence"
                    );
                }
            }
        }

        *self.online_members.write() = online;
    }
}

/// Entry point: called from veilid_service when we receive a SignedEnvelope
/// for a community where we're coordinator.
pub async fn handle_incoming_envelope(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    signed: SignedEnvelope,
) {
    // Refresh online members if cache is empty (lazy init)
    if relay.online_members.read().is_empty() {
        relay.refresh_online_members(state).await;
    }

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

    // 3. Read member info + roles for permission checking
    let (members, roles) = match read_members_and_roles(state, &relay.community_id).await {
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

    let now_secs = rekindle_utils::timestamp_secs();

    // 4. Route by type
    let signed_bytes = match serde_json::to_vec(&signed) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to re-serialize signed envelope");
            return;
        }
    };

    match envelope {
        CommunityEnvelope::ChatMessage { ref channel_id, .. } => {
            // Check SEND_MESSAGES permission
            let channel_overwrites = get_channel_overwrites(state, &relay.community_id, channel_id);
            let is_owner = is_community_owner(state, &relay.community_id, &signed.sender_pseudonym);
            let perms = calculate_permissions_v2(
                &sender.role_ids,
                &roles,
                &channel_overwrites,
                &signed.sender_pseudonym,
                is_owner,
                sender.timeout_until,
            );

            if !perms.has(Permissions::SEND_MESSAGES) {
                tracing::debug!(
                    pseudonym = %signed.sender_pseudonym,
                    channel = %channel_id,
                    "rejecting message: no SEND_MESSAGES permission"
                );
                return;
            }

            // Check timeout
            if timeout::is_timed_out(sender, &roles, now_secs) {
                tracing::debug!(
                    pseudonym = %signed.sender_pseudonym,
                    "rejecting message: member is timed out"
                );
                return;
            }

            // AutoMod check (including channel slowmode)
            let slowmode_seconds = get_channel_slowmode(state, &relay.community_id, channel_id);
            let automod_decision = {
                relay.automod.lock().check_envelope(
                    sender,
                    &envelope,
                    &roles,
                    Some(channel_id),
                    slowmode_seconds,
                    now_secs,
                )
            };
            match automod_decision {
                AutoModDecision::Allow => {}
                AutoModDecision::Block(reason) => {
                    tracing::info!(
                        pseudonym = %signed.sender_pseudonym,
                        channel = %channel_id,
                        reason = %reason,
                        "automod blocked message"
                    );
                    log_audit(state, relay, AuditAction::AutoModActionExecuted,
                        AuditTarget::Member(signed.sender_pseudonym.clone()),
                        vec![AuditChange { field: "action".into(), old_value: None, new_value: Some("block".into()) }],
                        Some(reason));
                    return;
                }
                AutoModDecision::Timeout { duration_secs, reason } => {
                    tracing::info!(
                        pseudonym = %signed.sender_pseudonym,
                        duration_secs,
                        reason = %reason,
                        "automod timed out member"
                    );
                    log_audit(state, relay, AuditAction::AutoModActionExecuted,
                        AuditTarget::Member(signed.sender_pseudonym.clone()),
                        vec![
                            AuditChange { field: "action".into(), old_value: None, new_value: Some("timeout".into()) },
                            AuditChange { field: "duration_secs".into(), old_value: None, new_value: Some(duration_secs.to_string()) },
                        ],
                        Some(reason));
                    return;
                }
                AutoModDecision::Alert { channel_id: alert_ch, reason } => {
                    tracing::info!(
                        pseudonym = %signed.sender_pseudonym,
                        alert_channel = %alert_ch,
                        reason = %reason,
                        "automod alert (message still relayed)"
                    );
                    log_audit(state, relay, AuditAction::AutoModActionExecuted,
                        AuditTarget::Member(signed.sender_pseudonym.clone()),
                        vec![AuditChange { field: "action".into(), old_value: None, new_value: Some("alert".into()) }],
                        Some(reason));
                }
            }

            // Fan out to all online members
            relay_to_members(state, relay, &signed_bytes, &signed.sender_pseudonym);
        }
        CommunityEnvelope::TypingIndicator { .. } => {
            // Fan out only (no persistence, ephemeral)
            relay_to_members(state, relay, &signed_bytes, &signed.sender_pseudonym);
        }
        CommunityEnvelope::PresenceUpdate {
            ref pseudonym_key,
            ref status,
            ..
        } => {
            // Invisible members: rewrite status to "offline" for fan-out.
            // The invisible member still receives all messages (they're in online_members).
            if status == "invisible" {
                let mut rewritten = envelope.clone();
                if let CommunityEnvelope::PresenceUpdate {
                    ref mut status, ..
                } = rewritten
                {
                    *status = "offline".to_string();
                }
                let rewritten_bytes =
                    serde_json::to_vec(&rewritten).unwrap_or_else(|_| signed_bytes.clone());
                relay_to_members(state, relay, &rewritten_bytes, pseudonym_key);
            } else {
                relay_to_members(state, relay, &signed_bytes, &signed.sender_pseudonym);
            }
        }
        CommunityEnvelope::Control(ref payload) => {
            handle_control(
                state,
                relay,
                sender,
                &roles,
                payload,
                &signed_bytes,
                &signed.sender_pseudonym,
            )
            .await;
        }
    }
}

/// Handle a control payload from a member.
async fn handle_control(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    sender: &MemberSummary,
    roles: &[rekindle_protocol::dht::community::types::RoleEntryV2],
    payload: &ControlPayload,
    signed_bytes: &[u8],
    sender_pseudonym: &str,
) {
    let is_owner = is_community_owner(state, &relay.community_id, sender_pseudonym);
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
            let should_reject = relay.raid.lock().should_reject_join();
            if should_reject {
                tracing::info!(
                    pseudonym = %sender_pseudonym,
                    "rejecting join: raid protection active"
                );
                return;
            }

            // Record the join for rate tracking
            let now_secs = rekindle_utils::timestamp_secs();
            let raid_actions = relay.raid.lock().record_join(now_secs);
            if let Some(actions) = raid_actions {
                tracing::warn!(
                    community = %relay.community_id,
                    actions = ?actions,
                    "raid detected — defensive actions activated"
                );
                execute_raid_actions(state, relay, &actions);
            }

            let onboard_state = state.clone();
            let onboard_community = relay.community_id.clone();
            let member_route = route_blob.clone();
            let relay_clone = Arc::clone(relay);
            let state_clone = state.clone();
            let signed_bytes_clone = signed_bytes.to_vec();
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
                        send_to_member(
                            &state_clone,
                            &relay_clone,
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
                    notify_invite_used(&state_clone, &relay_clone, code).await;
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
                        &relay_clone,
                        &onboard_community,
                        blob,
                    ).await;
                }

                // Check onboarding
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
                            send_to_member(
                                &state_clone,
                                &relay_clone,
                                blob,
                                questions_payload,
                            );
                        }
                        relay_to_members(
                            &state_clone,
                            &relay_clone,
                            &signed_bytes_clone,
                            &sender_clone,
                        );
                    }
                    Ok(None) => {
                        relay_to_members(
                            &state_clone,
                            &relay_clone,
                            &signed_bytes_clone,
                            &sender_clone,
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            community = %onboard_community,
                            error = %e,
                            "failed to check onboarding, admitting anyway"
                        );
                        relay_to_members(
                            &state_clone,
                            &relay_clone,
                            &signed_bytes_clone,
                            &sender_clone,
                        );
                    }
                }

                broadcast_system_message(
                    &state_clone,
                    &relay_clone,
                    &format!("{display_name_clone} joined the community"),
                );
            });
        }

        // Member leave — relay + system message
        ControlPayload::MemberLeave { ref pseudonym_key } => {
            let who = pseudonym_key.clone();
            relay_to_members(state, relay, signed_bytes, sender_pseudonym);
            broadcast_system_message(state, relay, &format!("{who} left the community"));
        }

        // Moderation - requires KICK_MEMBERS
        ControlPayload::Kick { target_pseudonym, .. } => {
            if base_perms.has(Permissions::KICK_MEMBERS) {
                let target = target_pseudonym.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                broadcast_system_message(state, relay, &format!("{target} was kicked"));
                log_audit(state, relay, AuditAction::MemberKick, AuditTarget::Member(target), vec![], None);
            }
        }
        ControlPayload::Ban { target_pseudonym, .. } => {
            if base_perms.has(Permissions::BAN_MEMBERS) {
                let target = target_pseudonym.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                broadcast_system_message(state, relay, &format!("{target} was banned"));
                log_audit(state, relay, AuditAction::MemberBan, AuditTarget::Member(target), vec![], None);
            }
        }
        ControlPayload::Unban { target_pseudonym, .. } => {
            if base_perms.has(Permissions::BAN_MEMBERS) {
                let target = target_pseudonym.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::MemberUnban, AuditTarget::Member(target), vec![], None);
            }
        }
        ControlPayload::TimeoutMember { target_pseudonym, duration_seconds, .. } => {
            if base_perms.has(Permissions::MODERATE_MEMBERS) {
                let target = target_pseudonym.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::MemberTimeout, AuditTarget::Member(target), vec![
                    AuditChange { field: "duration_seconds".into(), old_value: None, new_value: Some(duration_seconds.to_string()) },
                ], None);
            }
        }
        ControlPayload::RemoveTimeout { target_pseudonym, .. } => {
            if base_perms.has(Permissions::MODERATE_MEMBERS) {
                let target = target_pseudonym.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::MemberTimeoutRemove, AuditTarget::Member(target), vec![], None);
            }
        }

        // Channel management - requires MANAGE_CHANNELS
        ControlPayload::CreateChannel { name, .. } => {
            if base_perms.has(Permissions::MANAGE_CHANNELS) {
                let ch_name = name.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::ChannelCreate, AuditTarget::Community, vec![
                    AuditChange { field: "name".into(), old_value: None, new_value: Some(ch_name) },
                ], None);
            }
        }
        ControlPayload::DeleteChannel { channel_id, .. } => {
            if base_perms.has(Permissions::MANAGE_CHANNELS) {
                let ch_id = channel_id.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::ChannelDelete, AuditTarget::Channel(ch_id), vec![], None);
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
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
            }
        }

        // Role management + channel overwrites - requires MANAGE_ROLES
        ControlPayload::CreateRole { name, .. } => {
            if base_perms.has(Permissions::MANAGE_ROLES) {
                let role_name = name.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::RoleCreate, AuditTarget::Community, vec![
                    AuditChange { field: "name".into(), old_value: None, new_value: Some(role_name) },
                ], None);
            }
        }
        ControlPayload::DeleteRole { role_id, .. } => {
            if base_perms.has(Permissions::MANAGE_ROLES) {
                let rid = *role_id;
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::RoleDelete, AuditTarget::Role(rid), vec![], None);
            }
        }
        ControlPayload::AssignRole { target_pseudonym, role_id, .. }
        | ControlPayload::UnassignRole { target_pseudonym, role_id, .. } => {
            if base_perms.has(Permissions::MANAGE_ROLES) {
                let target = target_pseudonym.clone();
                let rid = *role_id;
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::MemberRoleUpdate, AuditTarget::Member(target), vec![
                    AuditChange { field: "role_id".into(), old_value: None, new_value: Some(rid.to_string()) },
                ], None);
            }
        }
        ControlPayload::EditRole { .. }
        | ControlPayload::SetChannelOverwrite { .. }
        | ControlPayload::DeleteChannelOverwrite { .. } => {
            if base_perms.has(Permissions::MANAGE_ROLES) {
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
            }
        }

        // All other control payloads — permission checks + relay
        other => handle_control_extended(state, relay, base_perms, other, signed_bytes, sender_pseudonym).await,
    }
}

/// Extended control payload handling (split from `handle_control` for clippy line limit).
async fn handle_control_extended(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    base_perms: Permissions,
    payload: &ControlPayload,
    signed_bytes: &[u8],
    sender_pseudonym: &str,
) {
    match payload {
        // Community metadata, game servers - requires MANAGE_COMMUNITY
        ControlPayload::UpdateCommunity { .. }
        | ControlPayload::ListInvites
        | ControlPayload::AddGameServer { .. }
        | ControlPayload::RemoveGameServer { .. } => {
            if base_perms.has(Permissions::MANAGE_COMMUNITY) {
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
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
                    &relay.community_id,
                    &creator,
                    code,
                    *max_uses,
                    *expires_in_seconds,
                ).await {
                    Ok(entry) => {
                        // Broadcast InviteCreated to all members
                        let broadcast = ControlPayload::InviteCreated {
                            code: entry.code.clone(),
                            created_by: entry.created_by.clone(),
                            max_uses: if entry.max_uses > 0 { Some(entry.max_uses) } else { None },
                            uses: entry.use_count,
                            expires_at: entry.expires_at,
                            created_at: entry.created_at,
                        };
                        let envelope = CommunityEnvelope::Control(broadcast);
                        if let Ok(bytes) = serde_json::to_vec(&envelope) {
                            relay_to_members(state, relay, &bytes, "");
                        }
                        tracing::info!(
                            community = %relay.community_id,
                            code = %entry.code,
                            "invite created and persisted to manifest"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            community = %relay.community_id,
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
                match revoke_invite_entry(state, &relay.community_id, code).await {
                    Ok(()) => {
                        // Broadcast InviteRevoked to all members
                        let broadcast = ControlPayload::InviteRevoked { code: code.clone() };
                        let envelope = CommunityEnvelope::Control(broadcast);
                        if let Ok(bytes) = serde_json::to_vec(&envelope) {
                            relay_to_members(state, relay, &bytes, "");
                        }
                        tracing::info!(
                            community = %relay.community_id,
                            code = %code,
                            "invite revoked"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            community = %relay.community_id,
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
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
            }
        }

        // Reactions
        ControlPayload::AddReaction { .. } | ControlPayload::RemoveReaction { .. } => {
            if base_perms.has(Permissions::ADD_REACTIONS) {
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
            }
        }

        // Message management (pin/unpin/delete) - requires MANAGE_MESSAGES
        ControlPayload::PinMessage { .. }
        | ControlPayload::UnpinMessage { .. }
        | ControlPayload::DeleteMessage { .. } => {
            if base_perms.has(Permissions::MANAGE_MESSAGES) {
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
            }
        }

        // Onboarding answers — process and assign roles
        ControlPayload::SubmitOnboardingAnswers { answers } => {
            let onboard_state = state.clone();
            let onboard_community = relay.community_id.clone();
            let onboard_answers = answers.clone();
            let onboard_pseudonym = sender_pseudonym.to_string();
            let relay_clone = Arc::clone(relay);
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
                                if let Ok(bytes) = serde_json::to_vec(&envelope) {
                                    relay_to_members(&state_clone, &relay_clone, &bytes, "");
                                }
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

        // Read-only operations & broadcast variants - relay to all
        _ => {
            relay_to_members(state, relay, signed_bytes, sender_pseudonym);
        }
    }
}

/// Send a control payload to a specific member by their route blob.
fn send_to_member(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    target_route_blob: &[u8],
    payload: ControlPayload,
) {
    let Some(rc) = state_helpers::routing_context(state) else {
        tracing::debug!("no routing context for send_to_member");
        return;
    };

    let envelope = CommunityEnvelope::Control(payload);
    let envelope_bytes = match serde_json::to_vec(&envelope) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize control payload");
            return;
        }
    };

    let blob = target_route_blob.to_vec();
    let community_id = relay.community_id.clone();
    tokio::spawn(async move {
        match rc.api().import_remote_private_route(blob) {
            Ok(route_id) => {
                if let Err(e) = rc
                    .app_message(veilid_core::Target::RouteId(route_id), envelope_bytes)
                    .await
                {
                    tracing::debug!(
                        community = %community_id,
                        error = %e,
                        "send_to_member delivery failed"
                    );
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "failed to import member route for send_to_member");
            }
        }
    });
}

/// Fan out envelope bytes to all online members except the sender.
fn relay_to_members(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    envelope_bytes: &[u8],
    exclude_pseudonym: &str,
) {
    // Clone routing context (parking_lot !Send)
    let Some(rc) = state_helpers::routing_context(state) else {
        tracing::debug!("no routing context for fan-out");
        return;
    };

    let members = relay.online_members.read().clone();

    for (pseudonym, route_blob) in &members {
        if pseudonym == exclude_pseudonym {
            continue;
        }
        let rc = rc.clone();
        let data = envelope_bytes.to_vec();
        let blob = route_blob.clone();
        let semaphore = relay.fan_out_semaphore.clone();
        tokio::spawn(async move {
            let _permit = semaphore.acquire().await;
            match rc.api().import_remote_private_route(blob) {
                Ok(route_id) => {
                    if let Err(e) = rc
                        .app_message(veilid_core::Target::RouteId(route_id), data)
                        .await
                    {
                        tracing::debug!(error = %e, "fan-out delivery failed");
                    }
                }
                Err(e) => {
                    tracing::debug!(error = %e, "failed to import member route for fan-out");
                }
            }
        });
    }
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

/// Broadcast a system message to all online members (join/leave/kick/ban events).
fn broadcast_system_message(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    body: &str,
) {
    let payload = ControlPayload::SystemMessage {
        body: body.to_string(),
        timestamp: rekindle_utils::timestamp_secs(),
    };
    let envelope = CommunityEnvelope::Control(payload);
    if let Ok(bytes) = serde_json::to_vec(&envelope) {
        relay_to_members(state, relay, &bytes, "");
    }
}

/// Execute raid defense actions triggered by the raid detector.
fn execute_raid_actions(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    actions: &[rekindle_protocol::dht::community::automod::RaidAction],
) {
    use rekindle_protocol::dht::community::automod::RaidAction;

    for action in actions {
        match action {
            RaidAction::PauseInvites | RaidAction::RestrictNewMembers => {
                // Handled by RaidDetector state flags — checked in should_reject_join()
            }
            RaidAction::AlertOwners => {
                // Broadcast a raid alert to the community owner
                let alert_payload =
                    ControlPayload::RaidAlert { active: true };
                let envelope = CommunityEnvelope::Control(alert_payload);
                if let Ok(bytes) = serde_json::to_vec(&envelope) {
                    relay_to_members(state, relay, &bytes, "");
                }
                log_audit(
                    state,
                    relay,
                    AuditAction::AutoModActionExecuted,
                    AuditTarget::Community,
                    vec![AuditChange {
                        field: "raid_action".into(),
                        old_value: None,
                        new_value: Some("alert_owners".into()),
                    }],
                    Some("raid detected".into()),
                );
            }
            RaidAction::LockdownChannels => {
                // Broadcast a lockdown notification to all members.
                // Members restrict SEND_MESSAGES client-side for non-admins.
                let lockdown_payload =
                    ControlPayload::ChannelLockdown { locked: true };
                let envelope = CommunityEnvelope::Control(lockdown_payload);
                if let Ok(bytes) = serde_json::to_vec(&envelope) {
                    relay_to_members(state, relay, &bytes, "");
                }
                log_audit(
                    state,
                    relay,
                    AuditAction::AutoModActionExecuted,
                    AuditTarget::Community,
                    vec![AuditChange {
                        field: "raid_action".into(),
                        old_value: None,
                        new_value: Some("lockdown_channels".into()),
                    }],
                    Some("raid detected".into()),
                );
            }
        }
    }
}

/// Get the channel's slowmode setting (seconds) from state.
fn get_channel_slowmode(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<u32> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|cs| cs.channels.iter().find(|c| c.id == channel_id))
        .and_then(|ch| ch.slowmode_seconds)
}

/// Get channel permission overwrites from state.
fn get_channel_overwrites(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Vec<rekindle_protocol::dht::community::PermissionOverwrite> {
    let communities = state.communities.read();
    if let Some(cs) = communities.get(community_id) {
        if let Some(ch) = cs.channels.iter().find(|c| c.id == channel_id) {
            // Channel overwrites are stored in the v2 manifest channels,
            // not currently in the state ChannelInfo. Return empty for now.
            let _ = ch;
        }
    }
    Vec::new()
}

/// Fire-and-forget audit log entry (spawns async task).
fn log_audit(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    action: AuditAction,
    target: AuditTarget,
    changes: Vec<AuditChange>,
    reason: Option<String>,
) {
    let state = state.clone();
    let community_id = relay.community_id.clone();
    let logger = relay.audit_logger.clone();
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
async fn send_join_accepted(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
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

    send_to_member(state, relay, target_route_blob, payload);
}

/// Notify all members that an invite was used (for frontend tracking).
async fn notify_invite_used(
    state: &Arc<AppState>,
    relay: &Arc<RelayService>,
    code: &str,
) {
    // Read the updated invite to get the new use count
    let Some(rc) = state_helpers::routing_context(state) else {
        return;
    };
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);

    let manifest_key = {
        let communities = state.communities.read();
        match communities.get(&relay.community_id) {
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
        if let Ok(bytes) = serde_json::to_vec(&envelope) {
            relay_to_members(state, relay, &bytes, "");
        }
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

