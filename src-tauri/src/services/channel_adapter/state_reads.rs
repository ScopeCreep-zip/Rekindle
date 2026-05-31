//! Phase 23.D.4 — read-heavy state lookups extracted from
//! `deps_impl.rs` to keep the trait impl under the 500-LoC cap. Each
//! function takes `&ChannelAdapter` and returns the same shape the
//! trait method does.

use std::collections::HashMap;

use rekindle_channel::deps::{
    ChannelInfoSnapshot, ChannelWriteContext, MemberProfileSnapshot, RoleSnapshot,
    ThreadStateSnapshot,
};
use rekindle_channel::error::ChannelError;
use rekindle_governance::permissions::compute_permissions;
use rekindle_types::id::{ChannelId, PseudonymKey, ThreadId};

use crate::state::ChannelType;
use crate::state_helpers;

use super::ChannelAdapter;

fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 16])
}

pub(super) fn channel_info_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
) -> Option<ChannelInfoSnapshot> {
    let communities = adapter.state.communities.read();
    let community = communities.get(community_id)?;
    let channel = community.channels.iter().find(|c| c.id == channel_id)?;
    let is_forum = matches!(channel.channel_type, ChannelType::Forum);
    let last_send_at_ms = community.channel_last_send_at.get(channel_id).copied();
    Some(ChannelInfoSnapshot {
        channel_id: channel.id.clone(),
        channel_type: channel.channel_type.to_string(),
        slowmode_seconds: channel.slowmode_seconds,
        last_send_at_ms,
        mek_generation: channel.mek_generation,
        is_forum,
    })
}

pub(super) fn channel_write_context_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    channel_id: &str,
) -> Result<ChannelWriteContext, ChannelError> {
    let communities = adapter.state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or_else(|| ChannelError::CommunityNotFound(community_id.into()))?;
    let segment_index = community.my_segment_index.unwrap_or(0);
    let channel_id_bytes = hex_to_id_16(channel_id);
    let channel_id_typed = ChannelId(channel_id_bytes);

    let channel_key = if segment_index == 0 {
        community
            .channel_log_keys
            .get(channel_id)
            .cloned()
            .ok_or_else(|| ChannelError::Adapter("channel record key missing".into()))?
    } else {
        community
            .governance_state
            .as_ref()
            .and_then(|gov| {
                gov.channel_segment_records
                    .get(&(channel_id_typed, segment_index))
                    .map(|rec| rec.record_key.clone())
            })
            .ok_or_else(|| {
                ChannelError::Adapter(format!(
                    "no channel record yet for segment {segment_index} of channel {channel_id} — \
                     call ensure_channel_segment_record first"
                ))
            })?
    };

    Ok(ChannelWriteContext {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        channel_key,
        slot_keypair_str: community
            .slot_keypair
            .clone()
            .ok_or_else(|| ChannelError::Adapter("slot keypair missing".into()))?,
        slot_index: community
            .my_subkey_index
            .ok_or_else(|| ChannelError::Adapter("community subkey index missing".into()))?,
        segment_index,
    })
}

pub(super) fn thread_state_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    thread_id: &str,
) -> Option<ThreadStateSnapshot> {
    let gov = state_helpers::governance_state(&adapter.state, community_id)?;
    let thread = gov.threads.get(&ThreadId(hex_to_id_16(thread_id)))?;
    Some(ThreadStateSnapshot {
        thread_id: thread_id.to_string(),
        parent_channel_id_hex: hex::encode(thread.parent_channel_id.0),
        name: thread.name.clone(),
        thread_type: thread.thread_type.clone(),
        record_key: thread.record_key.clone(),
        invited: thread.invited.clone(),
        forum_tag: thread.forum_tag.clone(),
        auto_archive_seconds: thread.auto_archive_seconds,
        created_lamport: thread.created_lamport,
        archived_lamport: thread.archived_lamport,
        creator_pseudonym_hex: hex::encode(thread.creator.0),
    })
}

pub(super) fn member_profile_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    pseudonym_hex: &str,
) -> MemberProfileSnapshot {
    let communities = adapter.state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return MemberProfileSnapshot::default();
    };
    let display_name = community
        .member_profiles
        .get(pseudonym_hex)
        .and_then(|p| p.display_name.clone());
    let role_ids = community
        .member_roles
        .get(pseudonym_hex)
        .cloned()
        .unwrap_or_default();
    MemberProfileSnapshot {
        display_name,
        role_ids,
    }
}

pub(super) fn list_member_profiles_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
) -> HashMap<String, MemberProfileSnapshot> {
    let communities = adapter.state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return HashMap::new();
    };
    community
        .member_profiles
        .iter()
        .map(|(pseudonym_hex, profile)| {
            let role_ids = community
                .member_roles
                .get(pseudonym_hex)
                .cloned()
                .unwrap_or_default();
            (
                pseudonym_hex.clone(),
                MemberProfileSnapshot {
                    display_name: profile.display_name.clone(),
                    role_ids,
                },
            )
        })
        .collect()
}

pub(super) fn community_roles_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
) -> Vec<RoleSnapshot> {
    adapter
        .state
        .communities
        .read()
        .get(community_id)
        .map(|c| {
            c.roles
                .iter()
                .map(|r| RoleSnapshot {
                    id: r.id,
                    name: r.name.clone(),
                    mentionable: r.mentionable,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn compute_my_permissions_impl(adapter: &ChannelAdapter, community_id: &str) -> u64 {
    let communities = adapter.state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return 0;
    };
    let Some(pseudo_hex) = community.my_pseudonym_key.as_ref() else {
        return 0;
    };
    let Ok(bytes) = hex::decode(pseudo_hex) else {
        return 0;
    };
    let Ok(arr): Result<[u8; 32], _> = bytes.as_slice().try_into() else {
        return 0;
    };
    let Some(gov) = community.governance_state.as_ref() else {
        return 0;
    };
    compute_permissions(
        &PseudonymKey(arr),
        None,
        gov,
        rekindle_utils::timestamp_secs(),
    )
}
