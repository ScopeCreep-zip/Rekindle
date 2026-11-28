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
    let created = prepare_thread(
        deps,
        community_id,
        channel_id,
        name,
        starter_message_id,
        forum_tag,
        auto_archive_override,
    )?;
    deps.write_governance_entry(community_id, created.entry)
        .await?;
    deps.persist_thread_row(community_id, &created.row).await?;
    Ok(created.row.id)
}

pub struct ThreadCreation {
    pub entry: GovernanceEntry,
    pub row: ThreadInfoSnapshot,
}

/// Build the governance entry and its local metadata before writing either.
pub(super) fn prepare_thread<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    name: &str,
    starter_message_id: &str,
    forum_tag: Option<String>,
    auto_archive_override: Option<u64>,
) -> Result<ThreadCreation, ChannelError> {
    let thread_id = rekindle_utils::random::id_bytes_16();
    let thread_type = parent_thread_type(deps, community_id, channel_id)?;
    let auto_archive_seconds = match auto_archive_override {
        Some(secs) => validate_auto_archive_seconds(secs)?,
        None => default_auto_archive_seconds(&thread_type),
    };
    let creator_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    let created_at = deps.now_secs();
    Ok(ThreadCreation {
        entry: GovernanceEntry::ThreadCreated {
            thread_id: ThreadId(thread_id),
            parent_channel_id: ChannelId(hex_to_id_16(channel_id)),
            name: name.to_owned(),
            thread_type,
            invited: Vec::new(),
            forum_tag: forum_tag.clone(),
            auto_archive_seconds,
        },
        row: ThreadInfoSnapshot {
            id: hex::encode(thread_id),
            channel_id: channel_id.to_owned(),
            name: name.to_owned(),
            starter_message_id: starter_message_id.to_owned(),
            creator_pseudonym,
            forum_tag,
            created_at,
            archived: false,
            auto_archive_seconds: u32::try_from(auto_archive_seconds).unwrap_or(u32::MAX),
            last_message_at: created_at,
            message_count: 0,
        },
    })
}

/// Phase 19.e — archive a thread (writes a `ThreadArchived` governance entry).
pub async fn archive_thread<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
) -> Result<(), ChannelError> {
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ThreadArchived {
            thread_id: ThreadId(hex_to_id_16(thread_id)),
        },
    )
    .await
}

/// Reopen `thread_id`: a `ThreadUnarchived` entry, later than its archive.
///
/// # Errors
/// We may neither manage threads nor be its creator, or the entry failed.
pub async fn unarchive_thread<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
) -> Result<(), ChannelError> {
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ThreadUnarchived {
            thread_id: ThreadId(hex_to_id_16(thread_id)),
        },
    )
    .await
}

// ---------- private helpers ----------
