//! Thread creation and archival.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, ThreadId};

use super::policy::{default_auto_archive_seconds, validate_auto_archive_seconds};
use super::support::{hex_to_id_16, parent_thread_type};
use crate::deps::{ChannelMessagingDeps, ThreadInfoSnapshot};
use crate::error::ChannelError;

/// Phase 19.e — public create_thread entry. Mirrors src-tauri shape.
pub async fn create_thread<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    name: &str,
    starter_message_id: &str,
    forum_tag: Option<String>,
    auto_archive_override: Option<u64>,
) -> Result<String, ChannelError> {
    let thread_id_bytes: [u8; 16] = rand::random();
    let thread_id = hex::encode(thread_id_bytes);
    let thread_type = parent_thread_type(deps, community_id, channel_id)?;
    let auto_archive_seconds = match auto_archive_override {
        Some(secs) => validate_auto_archive_seconds(secs)?,
        None => default_auto_archive_seconds(&thread_type),
    };
    let lamport = deps.increment_lamport(community_id);
    let forum_tag_for_entry = forum_tag.clone();

    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ThreadCreated {
            thread_id: ThreadId(thread_id_bytes),
            parent_channel_id: ChannelId(hex_to_id_16(channel_id)),
            name: name.to_string(),
            thread_type,
            record_key: None,
            invited: Vec::new(),
            forum_tag: forum_tag_for_entry,
            auto_archive_seconds,
            lamport,
        },
    )
    .await?;

    let creator_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;

    deps.persist_thread_row(
        community_id,
        &ThreadInfoSnapshot {
            id: thread_id.clone(),
            channel_id: channel_id.to_string(),
            name: name.to_string(),
            starter_message_id: starter_message_id.to_string(),
            creator_pseudonym,
            forum_tag,
            created_at: rekindle_utils::timestamp_secs(),
            archived: false,
            auto_archive_seconds: u32::try_from(auto_archive_seconds).unwrap_or(u32::MAX),
            last_message_at: 0,
            message_count: 0,
        },
    )
    .await?;

    Ok(thread_id)
}

/// Phase 19.e — archive a thread (writes a `ThreadArchived` governance entry).
pub async fn archive_thread<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
) -> Result<(), ChannelError> {
    let lamport = deps.increment_lamport(community_id);
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ThreadArchived {
            thread_id: ThreadId(hex_to_id_16(thread_id)),
            lamport,
        },
    )
    .await
}

// ---------- private helpers ----------
