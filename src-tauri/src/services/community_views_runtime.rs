//! Phase 23.C — community-list DTO mappers lifted from
//! `commands/community/crud.rs`. Both handlers are pure AppState reads
//! that flatten a `HashMap<id, CommunityState>` into Vec<Info> / Vec<Detail>
//! DTOs for the frontend. Pulled out per Invariant 7 so the Tauri
//! command surface stays a thin delegation.

use crate::commands::community::types::{CommunityDetail, CommunityRoleDto};
use crate::state::SharedState;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channel_count: usize,
    pub my_role: Option<String>,
}

#[derive(Debug, serde::Serialize)]
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
    pub forum_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_speakers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_moderator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slowmode_seconds: Option<u32>,
    pub notification_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_sound_ref: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfoDto {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

pub fn list_communities_inner(state: &SharedState) -> Vec<CommunityInfo> {
    let communities = state.communities.read();
    communities
        .values()
        .map(|community| CommunityInfo {
            id: community.id.clone(),
            name: community.name.clone(),
            description: community.description.clone(),
            channel_count: community.channels.len(),
            my_role: Some(crate::state::display_role_name(
                &community.my_role_ids,
                &community.roles,
            )),
        })
        .collect()
}

pub fn list_community_details_inner(state: &SharedState) -> Vec<CommunityDetail> {
    let communities = state.communities.read();
    communities
        .values()
        .map(|community| CommunityDetail {
            id: community.id.clone(),
            name: community.name.clone(),
            description: community.description.clone(),
            icon_hash: community.icon_hash.clone(),
            banner_hash: community.banner_hash.clone(),
            channels: community
                .channels
                .iter()
                .map(|channel| ChannelInfoDto {
                    id: channel.id.clone(),
                    name: channel.name.clone(),
                    channel_type: channel.channel_type.to_string(),
                    unread_count: channel.unread_count,
                    category_id: channel.category_id.clone(),
                    topic: channel.topic.clone(),
                    forum_tags: channel.forum_tags.clone(),
                    stage_speakers: channel.stage_speakers.clone(),
                    stage_moderator: channel.stage_moderator.clone(),
                    slowmode_seconds: channel.slowmode_seconds,
                    notification_level: channel.notification_level.clone(),
                    notification_sound_ref: channel.notification_sound_ref.clone(),
                })
                .collect(),
            categories: community
                .categories
                .iter()
                .map(|category| CategoryInfoDto {
                    id: category.id.clone(),
                    name: category.name.clone(),
                    sort_order: category.sort_order,
                })
                .collect(),
            my_role: Some(crate::state::display_role_name(
                &community.my_role_ids,
                &community.roles,
            )),
            my_role_ids: community.my_role_ids.clone(),
            roles: community.roles.iter().map(CommunityRoleDto::from).collect(),
            my_pseudonym_key: community.my_pseudonym_key.clone(),
            mek_generation: community.mek_generation,
            member_registry_key: community.member_registry_key.clone(),
            governance_key: community.governance_key.clone(),
            onboarding_complete: community.onboarding_complete,
            my_bio: community.my_bio.clone(),
            my_pronouns: community.my_pronouns.clone(),
            my_theme_color: community.my_theme_color,
            my_badges: community.my_badges.clone(),
        })
        .collect()
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionInfoDto {
    pub expression_id: String,
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    pub inline_data_base64: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
    /// Architecture §18.3 — present only on `kind == "soundboard"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_meta: Option<rekindle_types::expression::SoundboardMeta>,
    /// Architecture §18.1 line 2455 — uploader's per-community pseudonym (hex).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_pseudonym: Option<String>,
    /// Architecture §18.1 line 2456 — wall-clock seconds at upload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// Architecture §18.1 line 2459 — gates `USE_EXTERNAL_EMOJIS`.
    pub available_to_peers: bool,
}

pub fn list_expressions_inner(
    state: &SharedState,
    community_id: &str,
) -> Result<Vec<ExpressionInfoDto>, String> {
    let expressions = crate::services::community::list_expressions(state, community_id)?;
    Ok(expressions
        .into_iter()
        .map(|expression| ExpressionInfoDto {
            expression_id: expression.expression_id,
            name: expression.name,
            kind: expression.kind,
            content_hash: expression.content_hash,
            inline_data_base64: expression.inline_data_base64,
            media_type: expression.media_type,
            animated: expression.animated,
            tags: expression.tags,
            sound_meta: expression.sound_meta,
            creator_pseudonym: expression.creator_pseudonym,
            created_at: expression.created_at,
            available_to_peers: expression.available_to_peers,
        })
        .collect())
}
