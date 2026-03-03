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
    relay: &RelayService,
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

            // AutoMod check
            let automod_decision = {
                relay.automod.lock().check_envelope(
                    sender,
                    &envelope,
                    &roles,
                    Some(channel_id),
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
            );
        }
    }
}

/// Handle a control payload from a member.
fn handle_control(
    state: &Arc<AppState>,
    relay: &RelayService,
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
        // Join request - check raid protection + onboarding
        ControlPayload::MemberJoinRequest { .. } => {
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
            }

            // Check if onboarding is required — spawn async task
            let onboard_state = state.clone();
            let onboard_community = relay.community_id.clone();
            tokio::spawn(async move {
                match super::onboarding::check_onboarding(
                    &onboard_state,
                    &onboard_community,
                )
                .await
                {
                    Ok(Some(_questions_payload)) => {
                        tracing::debug!(
                            community = %onboard_community,
                            "onboarding questions will be sent to new member"
                        );
                        // Onboarding questions are sent via the JoinAccepted response
                        // from the coordinator, which is handled elsewhere
                    }
                    Ok(None) => {
                        // No onboarding required — member is admitted immediately
                    }
                    Err(e) => {
                        tracing::warn!(
                            community = %onboard_community,
                            error = %e,
                            "failed to check onboarding"
                        );
                    }
                }
            });

            relay_to_members(state, relay, signed_bytes, sender_pseudonym);
        }

        // Moderation - requires KICK_MEMBERS
        ControlPayload::Kick { target_pseudonym, .. } => {
            if base_perms.has(Permissions::KICK_MEMBERS) {
                let target = target_pseudonym.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
                log_audit(state, relay, AuditAction::MemberKick, AuditTarget::Member(target), vec![], None);
            }
        }
        ControlPayload::Ban { target_pseudonym, .. } => {
            if base_perms.has(Permissions::BAN_MEMBERS) {
                let target = target_pseudonym.clone();
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
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

        // Community metadata, invite management, game servers - requires MANAGE_COMMUNITY
        ControlPayload::UpdateCommunity { .. }
        | ControlPayload::RevokeInvite { .. }
        | ControlPayload::ListInvites
        | ControlPayload::AddGameServer { .. }
        | ControlPayload::RemoveGameServer { .. } => {
            if base_perms.has(Permissions::MANAGE_COMMUNITY) {
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
            }
        }

        // Invites - requires CREATE_INSTANT_INVITE
        ControlPayload::CreateInvite { .. } => {
            if base_perms.has(Permissions::CREATE_INSTANT_INVITE) {
                relay_to_members(state, relay, signed_bytes, sender_pseudonym);
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

        // Onboarding answers
        ControlPayload::SubmitOnboardingAnswers { answers } => {
            let onboard_state = state.clone();
            let onboard_community = relay.community_id.clone();
            let onboard_answers = answers.clone();
            let onboard_pseudonym = sender_pseudonym.to_string();
            tokio::spawn(async move {
                match super::onboarding::process_answers(
                    &onboard_state,
                    &onboard_community,
                    &onboard_answers,
                )
                .await
                {
                    Ok(role_ids) => {
                        tracing::info!(
                            community = %onboard_community,
                            pseudonym = %onboard_pseudonym,
                            roles = ?role_ids,
                            "onboarding answers processed"
                        );
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

/// Fan out envelope bytes to all online members except the sender.
fn relay_to_members(
    state: &Arc<AppState>,
    relay: &RelayService,
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
    relay: &RelayService,
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
