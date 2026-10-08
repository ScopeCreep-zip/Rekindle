use serde::Serialize;

pub use crate::channels::community_channel::RoleDto as CommunityRoleDto;
use crate::services::community_views_runtime::{CategoryInfoDto, ChannelInfoDto};

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
pub struct CommunityDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Architecture §32 Phase 5 Week 15 — community-level icon/banner
    /// references. Each is the BLAKE3 hex hash of a WebP-compressed
    /// image cached at `<app_data>/community_avatars/<id>/<hash>.webp`.
    /// Resolved to a `data:image/webp;base64,...` URL by
    /// `get_community_avatar_data_url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_hash: Option<String>,
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
    /// Our per-community profile values for prefilling the edit form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_pronouns: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_theme_color: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub my_badges: Vec<String>,
}
