use std::collections::HashMap;
use std::sync::Arc;

use crate::state::{AppState, ChannelInfo, ChannelType, RoleDefinition};
use crate::state_helpers;

use super::helpers::role_id_to_legacy_u32;

pub(super) fn join_status_label(state: &Arc<AppState>) -> &'static str {
    match state_helpers::identity_status(state).unwrap_or(crate::state::UserStatus::Online) {
        crate::state::UserStatus::Online => "online",
        crate::state::UserStatus::Away => "away",
        crate::state::UserStatus::Busy => "busy",
        crate::state::UserStatus::Offline | crate::state::UserStatus::Invisible => "offline",
    }
}

pub(super) fn build_channels(
    gov_state: &rekindle_governance::state::GovernanceState,
) -> Vec<ChannelInfo> {
    gov_state
        .channels
        .iter()
        .map(|(ch_id, ch)| ChannelInfo {
            id: hex::encode(ch_id.0),
            name: ch.name.clone(),
            channel_type: ch.channel_type.parse().unwrap_or(ChannelType::Text),
            unread_count: 0,
            category_id: ch.category_id.map(|category| hex::encode(category.0)),
            topic: ch.topic.clone().unwrap_or_default(),
            forum_tags: ch.forum_tags.clone(),
            stage_speakers: Vec::new(),
            stage_moderator: None,
            slowmode_seconds: ch.slowmode_seconds,
            nsfw: ch.nsfw.unwrap_or(false),
            message_record_key: Some(ch.record_key.clone()),
            mek_generation: 0,
            notification_level: "all".to_string(),
            notification_sound_ref: None,
            parent_voice_channel_id: ch.parent_voice_channel_id.map(|pv| hex::encode(pv.0)),
        })
        .collect()
}

pub(super) fn build_roles(
    gov_state: &rekindle_governance::state::GovernanceState,
) -> Vec<RoleDefinition> {
    gov_state
        .roles
        .iter()
        .map(|(role_id, role)| RoleDefinition {
            id: role_id_to_legacy_u32(role_id),
            name: role.name.clone(),
            color: role.color,
            permissions: role.permissions,
            position: role.position.cast_signed(),
            hoist: role.hoist,
            mentionable: role.mentionable,
            self_assignable: role.self_assignable,
            exclusion_group: None,
        })
        .collect()
}

pub(super) fn build_channel_log_keys(
    gov_state: &rekindle_governance::state::GovernanceState,
) -> HashMap<String, String> {
    gov_state
        .channels
        .iter()
        .map(|(ch_id, channel)| (hex::encode(ch_id.0), channel.record_key.clone()))
        .collect()
}
