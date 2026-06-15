//! Community governance — roles, moderation, invites, channels, MEK rotation.
//!
//! Split into domain-specific submodules:
//! - `roles.rs` — create, update, delete, assign, unassign
//! - `moderation.rs` — ban, unban, kick, timeout
//! - `channels.rs` — create (with shared SMPL record), delete, update
//! - `invites.rs` — create, revoke
//! - `mek.rs` — rotate, rekey, request, handle_request, receive_transfer
//!
//! Shared helpers (read/write DHT, keypair access, gossip notifications)
//! live here in mod.rs. The RPC dispatch (`handle_governance_op`) also lives
//! here since it calls methods from all submodules.

mod roles;
mod moderation;
mod channels;
mod invites;
mod mek;

use rekindle_types::dht_types::{
    BanEntry, ChannelEntry, InviteEntry, PinEntry, ReactionEntry, RoleEntry,
    MANIFEST_BANS, MANIFEST_CHANNELS, MANIFEST_EVENTS, MANIFEST_INVITES,
    MANIFEST_METADATA, MANIFEST_ONBOARDING, MANIFEST_PINS, MANIFEST_REACTIONS,
    MANIFEST_ROLES, MANIFEST_WELCOME,
};
use rekindle_types::gossip_payload::{GossipPayload, ControlPayload};
use rekindle_types::rpc_payload::{GovernanceOp, GovernanceRequest};

use crate::ChatError;
use super::CommunityService;

impl CommunityService {
    // ── Gossip notification helpers ─────────────────────────────

    /// Notify mesh peers of a governance manifest subkey change.
    pub(crate) async fn notify_governance_updated(&self, gov_key: &str, subkey: u32) {
        let _ = self.io.broadcast_gossip_dedup(gov_key, GossipPayload::Control(
            ControlPayload::GovernanceUpdated {
                governance_key: gov_key.into(), subkey_index: subkey, lamport_ts: 0,
            },
        )).await;
    }

    /// Notify mesh peers of a membership change.
    pub(crate) async fn notify_membership(&self, gov_key: &str, payload: ControlPayload) {
        let _ = self.io.broadcast_gossip_dedup(gov_key, GossipPayload::Control(payload)).await;
    }

    // ── Keypair access ──────────────────────────────────────────

    pub(crate) fn require_governance_keypair(&self, governance_key: &str) -> Result<Vec<u8>, ChatError> {
        let short = &governance_key[..12.min(governance_key.len())];
        self.vault
            .load_key(&rekindle_storage::keys::labels::governance_keypair(short))?
            .ok_or_else(|| ChatError::InsufficientPermissions {
                action: "governance write (no keypair)".into(),
            })
    }

    // ── Signed governance write ──────────────────────────────────

    pub(crate) async fn write_signed_governance(
        &self, gov_key: &str, subkey: u32, data: &[u8],
    ) -> Result<(), ChatError> {
        let gov_keypair = self.require_governance_keypair(gov_key)?;
        let pseudonym_hex = self.io.pseudonym_hex(gov_key)?;
        let pseudonym_seed = self.io.pseudonym_seed(gov_key)?;
        let pseudonym_kp = rekindle_identity::SigningKeypair::from_seed(&pseudonym_seed)
            .map_err(|e| ChatError::Internal(format!("pseudonym keypair: {e}")))?;

        let lamport = {
            let mut meta = self.session_meta.write();
            if let Some(m) = meta.communities.get_mut(gov_key) {
                m.lamport_counter += 1;
                m.lamport_counter
            } else { 1 }
        };

        let mut payload = rekindle_types::dht_types::GovernanceSubkeyPayload {
            author_pseudonym: pseudonym_hex,
            subkey_index: subkey,
            data: data.to_vec(),
            lamport_ts: lamport,
            signature: Vec::new(),
        };
        let sig = pseudonym_kp.sign_raw(&payload.signing_bytes());
        payload.signature = sig.to_vec();

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| ChatError::Serialization(format!("governance payload: {e}")))?;
        self.io.open_and_write(gov_key, subkey, &payload_bytes, Some(&gov_keypair), crate::io::Confirm::Accepted).await?;
        Ok(())
    }

    pub(crate) async fn read_verified_governance(
        &self, gov_key: &str, subkey: u32,
    ) -> Result<Option<Vec<u8>>, ChatError> {
        let raw = self.io.open_and_read(gov_key, subkey, true).await?;
        let Some(bytes) = raw else { return Ok(None) };
        if bytes.is_empty() { return Ok(None); }

        // Try parsing as signed payload
        if let Ok(payload) = serde_json::from_slice::<rekindle_types::dht_types::GovernanceSubkeyPayload>(&bytes) {
            if !payload.signature.is_empty() {
                if let Ok(pub_bytes) = hex::decode(&payload.author_pseudonym) {
                    if pub_bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&pub_bytes);
                        if let Ok(sig_arr) = <[u8; 64]>::try_from(payload.signature.as_slice()) {
                            if rekindle_identity::verify_signature(&arr, &payload.signing_bytes(), &sig_arr).is_err() {
                                tracing::warn!(subkey, author = &payload.author_pseudonym[..12.min(payload.author_pseudonym.len())], "governance signature FAILED");
                            }
                        }
                    }
                }
            }
            return Ok(Some(payload.data));
        }

        // Fallback: unsigned raw data (backwards compat)
        Ok(Some(bytes))
    }

    // ── DHT read helpers ────────────────────────────────────────

    pub(crate) async fn read_roles(&self, gov_key: &str) -> Result<Vec<RoleEntry>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_ROLES).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("roles: {e}")))
    }

    pub(crate) async fn read_bans(&self, gov_key: &str) -> Result<Vec<BanEntry>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_BANS).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("bans: {e}")))
    }

    pub(crate) async fn read_invites(&self, gov_key: &str) -> Result<Vec<InviteEntry>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_INVITES).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("invites: {e}")))
    }

    /// Read only the mek_rotation_interval_hours from governance metadata.
    /// Returns None if metadata can't be read or parsed.
    pub(crate) async fn read_governance_metadata_interval(&self, gov_key: &str) -> Option<u32> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_METADATA).await.ok()??;
        let metadata: rekindle_types::dht_types::CommunityMetadata = serde_json::from_slice(&raw).ok()?;
        Some(metadata.mek_rotation_interval_hours)
    }

    pub(crate) async fn read_channels(&self, gov_key: &str) -> Result<Vec<ChannelEntry>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_CHANNELS).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("channels: {e}")))
    }

    pub(crate) async fn read_pins(&self, gov_key: &str) -> Result<Vec<PinEntry>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_PINS).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("pins: {e}")))
    }

    pub(crate) async fn read_reactions(&self, gov_key: &str) -> Result<Vec<ReactionEntry>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_REACTIONS).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("reactions: {e}")))
    }

    pub(crate) async fn read_events(&self, gov_key: &str) -> Result<Vec<rekindle_types::gossip_payload::CommunityEvent>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_EVENTS).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("events: {e}")))
    }

    pub(crate) async fn append_audit_entry(
        &self,
        gov_key: &str,
        action: &str,
        actor_pseudonym: &str,
        target: Option<&str>,
        details: Option<&str>,
    ) {
        let entry = rekindle_types::dht_types::AuditLogEntry {
            action: action.to_string(),
            actor_pseudonym: actor_pseudonym.to_string(),
            target: target.map(String::from),
            timestamp: crate::time::timestamp_ms(),
            details: details.map(String::from),
        };
        let mut log = self.read_verified_governance(gov_key, rekindle_types::dht_types::MANIFEST_AUDIT_LOG_KEY).await
            .ok().flatten()
            .and_then(|raw| serde_json::from_slice::<Vec<rekindle_types::dht_types::AuditLogEntry>>(&raw).ok())
            .unwrap_or_default();
        log.push(entry);
        if log.len() > 1000 { log.drain(..log.len() - 1000); }
        if let Ok(bytes) = serde_json::to_vec(&log) {
            let _ = self.write_signed_governance(gov_key, rekindle_types::dht_types::MANIFEST_AUDIT_LOG_KEY, &bytes).await;
        }
    }

    pub(crate) async fn read_onboarding_config(&self, gov_key: &str) -> Result<Option<rekindle_types::dht_types::OnboardingConfig>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_ONBOARDING).await?;
        match raw {
            Some(bytes) if !bytes.is_empty() => Ok(Some(
                serde_json::from_slice(&bytes).map_err(|e| ChatError::Deserialization(format!("onboarding config: {e}")))?
            )),
            _ => Ok(None),
        }
    }

    pub(crate) async fn write_onboarding_config(&self, gov_key: &str, config: &rekindle_types::dht_types::OnboardingConfig) -> Result<(), ChatError> {
        let bytes = serde_json::to_vec(config).map_err(|e| ChatError::Serialization(format!("onboarding config: {e}")))?;
        self.write_signed_governance(gov_key, MANIFEST_ONBOARDING, &bytes).await
    }

    pub(crate) async fn read_welcome_screen(&self, gov_key: &str) -> Result<Option<rekindle_types::dht_types::WelcomeScreen>, ChatError> {
        let raw = self.read_verified_governance(gov_key, MANIFEST_WELCOME).await?;
        match raw {
            Some(bytes) if !bytes.is_empty() => Ok(Some(
                serde_json::from_slice(&bytes).map_err(|e| ChatError::Deserialization(format!("welcome screen: {e}")))?
            )),
            _ => Ok(None),
        }
    }

    pub(crate) async fn write_welcome_screen(&self, gov_key: &str, screen: &rekindle_types::dht_types::WelcomeScreen) -> Result<(), ChatError> {
        let bytes = serde_json::to_vec(screen).map_err(|e| ChatError::Serialization(format!("welcome screen: {e}")))?;
        self.write_signed_governance(gov_key, MANIFEST_WELCOME, &bytes).await
    }

    pub(crate) async fn read_audit_log(&self, gov_key: &str, limit: u32) -> Result<Vec<rekindle_types::dht_types::AuditLogEntry>, ChatError> {
        let raw = self.read_verified_governance(gov_key, rekindle_types::dht_types::MANIFEST_AUDIT_LOG_KEY).await?
            .unwrap_or_else(|| b"[]".to_vec());
        let mut entries: Vec<rekindle_types::dht_types::AuditLogEntry> = serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("audit log: {e}")))?;
        entries.reverse();
        entries.truncate(limit as usize);
        Ok(entries)
    }

    // ── RPC dispatch (called from events/router.rs) ─────────────

    /// Handle an inbound governance RPC from a community member.
    pub async fn handle_governance_op(
        &self,
        sender: &str,
        req: GovernanceRequest,
    ) -> Result<(), ChatError> {
        let gov_key = &req.governance_key;
        match req.operation {
            GovernanceOp::RegisterChannelRecord { .. } => Ok(()),
            GovernanceOp::Ban { target_pseudonym, reason } =>
                self.ban_member(gov_key, &target_pseudonym, reason.as_deref(), sender).await,
            GovernanceOp::Kick { target_pseudonym } =>
                self.kick_member(gov_key, &target_pseudonym).await,
            GovernanceOp::Unban { target_pseudonym } =>
                self.unban_member(gov_key, &target_pseudonym).await,
            GovernanceOp::Timeout { target_pseudonym, duration_seconds, .. } =>
                self.timeout_member(gov_key, &target_pseudonym, duration_seconds).await,
            GovernanceOp::ApproveJoin { target_pseudonym } =>
                self.approve_member(gov_key, &target_pseudonym).await.map(|_| ()),
            GovernanceOp::RejectJoin { target_pseudonym, reason } =>
                self.reject_member(gov_key, &target_pseudonym, &reason).await,
            GovernanceOp::CreateChannel { name, kind, .. } =>
                self.create_channel(gov_key, &name, &kind).await.map(|_| ()),
            GovernanceOp::DeleteChannel { channel_id } =>
                self.delete_channel(gov_key, &channel_id).await,
            GovernanceOp::UpdateChannel { channel_id, name, topic } =>
                self.update_channel(gov_key, &channel_id, name.as_deref(), topic.as_deref()).await.map(|_| ()),
            GovernanceOp::CreateRole { name, permissions, color, position } =>
                self.create_role(gov_key, &name, permissions, color, position).await.map(|_| ()),
            GovernanceOp::UpdateRole { role_id, name, permissions, color } =>
                self.update_role(gov_key, role_id, name.as_deref(), permissions, color).await.map(|_| ()),
            GovernanceOp::DeleteRole { role_id } =>
                self.delete_role(gov_key, role_id).await,
            GovernanceOp::AssignRole { member_pseudonym, role_id } =>
                self.assign_role(gov_key, &member_pseudonym, role_id).await,
            GovernanceOp::UnassignRole { member_pseudonym, role_id } =>
                self.unassign_role(gov_key, &member_pseudonym, role_id).await,
            GovernanceOp::RotateMek { channel_id } =>
                self.rotate_mek(gov_key, &channel_id).await.map(|_| ()),
            GovernanceOp::TransferOwnership { new_owner_pseudonym } =>
                self.transfer_ownership(gov_key, &new_owner_pseudonym).await,
        }
    }
}
