//! Community governance — roles, moderation, invites, channels, MEK rotation.
//!
//! Split into domain-specific submodules:
//! - `roles.rs` — create, update, delete, assign, unassign
//! - `moderation.rs` — ban, unban, kick, timeout
//! - `channels.rs` — create, delete, update, register_channel_record
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
    BanEntry, ChannelEntry, InviteEntry, RoleEntry,
    MANIFEST_BANS, MANIFEST_CHANNELS, MANIFEST_INVITES, MANIFEST_ROLES,
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

    // ── DHT read helpers ────────────────────────────────────────

    pub(crate) async fn read_roles(&self, gov_key: &str) -> Result<Vec<RoleEntry>, ChatError> {
        let raw = self.io.read_record(gov_key, MANIFEST_ROLES, true).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("roles: {e}")))
    }

    pub(crate) async fn read_bans(&self, gov_key: &str) -> Result<Vec<BanEntry>, ChatError> {
        let raw = self.io.read_record(gov_key, MANIFEST_BANS, true).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("bans: {e}")))
    }

    pub(crate) async fn read_invites(&self, gov_key: &str) -> Result<Vec<InviteEntry>, ChatError> {
        let raw = self.io.read_record(gov_key, MANIFEST_INVITES, true).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("invites: {e}")))
    }

    pub(crate) async fn read_channels(&self, gov_key: &str) -> Result<Vec<ChannelEntry>, ChatError> {
        let raw = self.io.read_record(gov_key, MANIFEST_CHANNELS, true).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw).map_err(|e| ChatError::Deserialization(format!("channels: {e}")))
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
            GovernanceOp::RegisterChannelRecord { member_pseudonym, channel_id, record_key } =>
                self.register_channel_record(gov_key, &member_pseudonym, &channel_id, &record_key).await,
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
