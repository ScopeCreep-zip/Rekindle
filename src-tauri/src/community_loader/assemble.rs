//! Phase 23.C — `CommunityState` / `ChannelInfo` / `RoleDefinition`
//! / `MemberProfileSnapshot` assembly from the raw DAO rows.
//!
//! Pure projection functions — no DB, no AppState, no Veilid; they
//! map a `CommunityLoaderRows` snapshot into a `CommunityState`.

use crate::state::{CategoryInfo, ChannelInfo, CommunityState, RoleDefinition};

use super::rows::{
    CategoryRow, ChannelRow, CommunityLoaderRows, CommunityRow, EventRsvpRow, MemberRow, RoleRow,
    SlowmodeRow,
};

pub fn channel_info_from_row(row: &ChannelRow) -> ChannelInfo {
    ChannelInfo {
        id: row.id.clone(),
        name: row.name.clone(),
        channel_type: row.channel_type.clone(),
        unread_count: 0,
        category_id: row.category_id.clone(),
        topic: row.topic.clone(),
        forum_tags: None,
        stage_speakers: Vec::new(),
        stage_moderator: None,
        slowmode_seconds: row.slowmode_seconds,
        nsfw: row.nsfw,
        message_record_key: row.message_record_key.clone(),
        mek_generation: row.mek_generation,
        notification_level: match row.notification_level {
            1 => "mentions".to_string(),
            2 => "nothing".to_string(),
            _ => "all".to_string(),
        },
        notification_sound_ref: row.notification_sound_ref.clone(),
        parent_voice_channel_id: row.parent_voice_channel_id.clone(),
    }
}

/// Project channels for one community, also extracting log keys and
/// per-channel sequence counters.
pub fn assemble_channels_for(
    community_id: &str,
    rows: &[ChannelRow],
) -> (
    Vec<ChannelInfo>,
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, u64>,
) {
    let mut log_keys = std::collections::HashMap::new();
    let mut sequences = std::collections::HashMap::new();
    let channels = rows
        .iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| {
            if let Some(ref lk) = r.log_key {
                log_keys.insert(r.id.clone(), lk.clone());
            }
            if r.my_sequence > 0 {
                sequences.insert(r.id.clone(), r.my_sequence);
            }
            channel_info_from_row(r)
        })
        .collect();
    (channels, log_keys, sequences)
}

pub fn assemble_roles_for(community_id: &str, rows: &[RoleRow]) -> Vec<RoleDefinition> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| RoleDefinition {
            id: r.role_id,
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position,
            hoist: r.hoist,
            mentionable: r.mentionable,
            self_assignable: r.self_assignable,
            exclusion_group: r.exclusion_group.clone(),
        })
        .collect()
}

pub fn assemble_categories_for(community_id: &str, rows: &[CategoryRow]) -> Vec<CategoryInfo> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| CategoryInfo {
            id: r.id.clone(),
            name: r.name.clone(),
            sort_order: r.sort_order,
        })
        .collect()
}

pub fn assemble_known_members_for(
    community_id: &str,
    rows: &[MemberRow],
) -> std::collections::HashSet<String> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| r.pseudonym_key.clone())
        .collect()
}

pub fn assemble_member_profiles_for(
    community_id: &str,
    rows: &[MemberRow],
) -> std::collections::HashMap<String, crate::state::MemberProfileSnapshot> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| {
            let badges: Vec<String> = serde_json::from_str(&r.badges_json).unwrap_or_default();
            let theme_color = r.theme_color.and_then(|c| u32::try_from(c).ok());
            (
                r.pseudonym_key.clone(),
                crate::state::MemberProfileSnapshot {
                    display_name: r.display_name.clone(),
                    bio: r.bio.clone(),
                    pronouns: r.pronouns.clone(),
                    theme_color,
                    badges,
                    avatar_ref: r.avatar_ref.clone(),
                    banner_ref: r.banner_ref.clone(),
                },
            )
        })
        .collect()
}

pub fn assemble_event_rsvps_for(
    community_id: &str,
    rows: &[EventRsvpRow],
) -> std::collections::HashMap<String, String> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| (r.event_id.clone(), r.status.clone()))
        .collect()
}

pub fn assemble_slowmode_for(
    community_id: &str,
    rows: &[SlowmodeRow],
) -> std::collections::HashMap<String, i64> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| (r.channel_id.clone(), r.last_send_ms))
        .collect()
}

pub fn build_community_state(
    community: &CommunityRow,
    rows: &CommunityLoaderRows,
) -> CommunityState {
    let (channels, channel_log_keys, channel_sequences) =
        assemble_channels_for(&community.id, &rows.channels);
    let my_role_ids: Vec<u32> =
        serde_json::from_str(&community.my_role_ids_json).unwrap_or_else(|_| vec![0, 1]);
    let roles = assemble_roles_for(&community.id, &rows.roles);
    CommunityState {
        id: community.id.clone(),
        name: community.name.clone(),
        description: community.description.clone(),
        icon_hash: community.icon_hash.clone(),
        banner_hash: community.banner_hash.clone(),
        channels,
        categories: assemble_categories_for(&community.id, &rows.categories),
        my_role_ids,
        roles,
        dht_owner_keypair: community.dht_owner_keypair.clone(),
        my_pseudonym_key: community.my_pseudonym_key.clone(),
        mek_generation: community.mek_generation,
        member_registry_key: community.member_registry_key.clone(),
        my_subkey_index: community.my_subkey_index,
        my_segment_index: community.my_segment_index,
        gossip: Some(crate::state::GossipOverlay::default()),
        slot_keypair: None,
        channel_log_keys,
        channel_sequences,
        pending_syncs: std::collections::HashMap::new(),
        watched_records: std::collections::HashSet::new(),
        record_sequences: std::collections::HashMap::new(),
        peer_sequences: std::collections::HashMap::new(),
        channel_last_send_at: assemble_slowmode_for(&community.id, &rows.slowmode),
        peer_reliability: std::collections::HashMap::new(),
        registry_owner_keypair: None,
        slot_seed: None,
        member_roles: std::collections::HashMap::new(),
        known_members: assemble_known_members_for(&community.id, &rows.members),
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
        open_community_records: crate::state::CommunityRecords::default(),
        my_event_rsvps: assemble_event_rsvps_for(&community.id, &rows.event_rsvps),
        event_rsvps_by_event: std::collections::HashMap::new(),
        onboarding_complete: community.onboarding_complete,
        governance_key: Some(community.id.clone()),
        governance_state: None,
        lamport_counter: 0,
        my_bio: None,
        my_pronouns: None,
        my_theme_color: None,
        my_badges: Vec::new(),
        my_avatar_ref: None,
        my_banner_ref: None,
        member_profiles: assemble_member_profiles_for(&community.id, &rows.members),
        recent_member_joins: std::collections::VecDeque::new(),
    }
}
