//! Governance delegation — roles, moderation, channels, invites.
//!
//! Every governance operation maps to a CommunityService method in
//! governance/mod.rs. The pseudonym derivation for `banned_by` uses
//! the caller's pseudonym from PlatformIO.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    // ── Roles ──────────────────────────────────────────────────────

    pub async fn create_role(
        &self, community: &str, name: &str, permissions: u64, color: u32, position: i32,
    ) -> Result<rekindle_types::dht_types::RoleEntry, ChatError> {
        self.community.create_role(community, name, permissions, color, position).await
    }

    pub async fn update_role(
        &self, community: &str, role_id: u32, name: Option<&str>, permissions: Option<u64>, color: Option<u32>,
    ) -> Result<rekindle_types::dht_types::RoleEntry, ChatError> {
        self.community.update_role(community, role_id, name, permissions, color).await
    }

    pub async fn delete_role(&self, community: &str, role_id: u32) -> Result<(), ChatError> {
        self.community.delete_role(community, role_id).await
    }

    pub async fn assign_role(
        &self, community: &str, member_pseudonym: &str, role_id: u32,
    ) -> Result<(), ChatError> {
        self.community.assign_role(community, member_pseudonym, role_id).await
    }

    pub async fn unassign_role(
        &self, community: &str, member_pseudonym: &str, role_id: u32,
    ) -> Result<(), ChatError> {
        self.community.unassign_role(community, member_pseudonym, role_id).await
    }

    pub async fn list_roles(
        &self, community: &str,
    ) -> Result<Vec<rekindle_types::dht_types::RoleEntry>, ChatError> {
        self.community.read_roles(community).await
    }

    // ── Moderation ─────────────────────────────────────────────────

    pub async fn kick_member(
        &self, community: &str, target_pseudonym: &str,
    ) -> Result<(), ChatError> {
        self.community.kick_member(community, target_pseudonym).await
    }

    pub async fn ban_member(
        &self, community: &str, target_pseudonym: &str, reason: Option<&str>,
    ) -> Result<(), ChatError> {
        let banned_by = self.io.pseudonym_hex(community)?;
        self.community.ban_member(community, target_pseudonym, reason, &banned_by).await
    }

    pub async fn unban_member(
        &self, community: &str, target_pseudonym: &str,
    ) -> Result<(), ChatError> {
        self.community.unban_member(community, target_pseudonym).await
    }

    pub async fn timeout_member(
        &self, community: &str, target_pseudonym: &str, duration_seconds: u64,
    ) -> Result<(), ChatError> {
        self.community.timeout_member(community, target_pseudonym, duration_seconds).await
    }

    pub async fn list_bans(
        &self, community: &str,
    ) -> Result<Vec<rekindle_types::dht_types::BanEntry>, ChatError> {
        self.community.read_bans(community).await
    }

    // ── Channels ───────────────────────────────────────────────────

    pub async fn create_channel(
        &self, community: &str, name: &str, kind: &str,
    ) -> Result<rekindle_types::dht_types::ChannelEntry, ChatError> {
        self.community.create_channel(community, name, kind).await
    }

    pub async fn delete_channel(
        &self, community: &str, channel_id: &str,
    ) -> Result<(), ChatError> {
        self.community.delete_channel(community, channel_id).await
    }

    pub async fn update_channel(
        &self, community: &str, channel_id: &str, name: Option<&str>, topic: Option<&str>,
    ) -> Result<rekindle_types::dht_types::ChannelEntry, ChatError> {
        self.community.update_channel(community, channel_id, name, topic).await
    }

    pub async fn list_channels(
        &self, community: &str,
    ) -> Result<Vec<rekindle_types::dht_types::ChannelEntry>, ChatError> {
        self.community.read_channels(community).await
    }

    // ── Invites ────────────────────────────────────────────────────

    pub async fn create_invite(
        &self, community: &str, max_uses: u32, expires_seconds: Option<u64>,
    ) -> Result<String, ChatError> {
        let created_by = self.io.pseudonym_hex(community)?;
        self.community.create_invite(community, &created_by, max_uses, expires_seconds).await
    }

    pub async fn list_invites(
        &self, community: &str,
    ) -> Result<Vec<rekindle_types::dht_types::InviteEntry>, ChatError> {
        self.community.read_invites(community).await
    }

    pub async fn revoke_invite(
        &self, community: &str, invite_code: &str,
    ) -> Result<(), ChatError> {
        self.community.revoke_invite(community, invite_code).await
    }
}
