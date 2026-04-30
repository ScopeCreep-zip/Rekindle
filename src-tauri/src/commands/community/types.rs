use serde::Serialize;

pub use crate::channels::community_channel::RoleDto as CommunityRoleDto;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDto {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub display_role: String,
    pub status: String,
    pub timeout_until: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channel_count: usize,
    pub my_role: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfoDto {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub unread_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub topic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slowmode_seconds: Option<u32>,
    pub notification_level: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietHoursSettingsDto {
    pub enabled: bool,
    pub start_hour: u8,
    pub end_hour: u8,
    pub utc_offset_minutes: i16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfoDto {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteCreatedDto {
    pub code: String,
    pub governance_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageResponse {
    pub status: String,
    pub message_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfoDto {
    pub code_hash: String,
    pub created_by: String,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub expires_at: Option<u64>,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedMessageInfoDto {
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntryInfoDto {
    pub action: String,
    pub actor_pseudonym: String,
    pub target: Option<String>,
    pub details: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BannedMemberInfo {
    pub pseudonym_key: String,
    pub display_name: String,
    pub banned_at: u64,
    pub reason: Option<String>,
    pub banned_by: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCountEntry {
    pub channel_id: String,
    pub unread_count: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GossipDiagnostics {
    pub community_id: String,
    pub has_gossip: bool,
    pub gossip_peer_count: usize,
    pub online_member_count: usize,
    pub known_member_count: usize,
    pub needs_initial_sync: bool,
    pub lamport_counter: u64,
    pub has_route_blob: bool,
    pub my_pseudonym_key: Option<String>,
    pub my_subkey_index: Option<u32>,
    pub has_slot_keypair: bool,
    pub has_slot_seed: bool,
    pub has_mek: bool,
    pub governance_key: Option<String>,
    pub gossip_peer_keys: Vec<String>,
    pub online_member_keys: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channels: Vec<ChannelInfoDto>,
    pub categories: Vec<CategoryInfoDto>,
    pub my_role: Option<String>,
    pub my_role_ids: Vec<u32>,
    pub roles: Vec<CommunityRoleDto>,
    pub my_pseudonym_key: Option<String>,
    pub mek_generation: u64,
    pub member_registry_key: Option<String>,
    pub governance_key: Option<String>,
    pub onboarding_complete: bool,
}
