//! Thread listing.

use super::policy::is_thread_archived;
use super::support::thread_activity;
use crate::deps::{ChannelMessagingDeps, ThreadInfoSnapshot};
use crate::error::ChannelError;

/// Phase 19.e — list threads (with archival status computed via
/// `is_thread_archived`).
pub async fn list_threads<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<ThreadInfoSnapshot>, ChannelError> {
    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    let mut out = Vec::new();
    let now = rekindle_utils::timestamp_secs();

    for (thread_id, thread) in gov_state
        .threads
        .iter()
        .filter(|(_, thread)| hex::encode(thread.parent_channel_id.0) == channel_id)
    {
        let thread_id_hex = hex::encode(thread_id.0);
        let mut dto = deps
            .load_thread_metadata(community_id, &thread_id_hex)
            .await
            .unwrap_or_else(|| ThreadInfoSnapshot {
                id: thread_id_hex.clone(),
                channel_id: channel_id.to_string(),
                name: thread.name.clone(),
                starter_message_id: String::new(),
                creator_pseudonym: hex::encode(thread.creator.0),
                forum_tag: thread.forum_tag.clone(),
                created_at: thread.created_lamport,
                archived: false,
                auto_archive_seconds: u32::try_from(thread.auto_archive_seconds)
                    .unwrap_or(u32::MAX),
                last_message_at: 0,
                message_count: 0,
            });
        dto.name.clone_from(&thread.name);
        dto.creator_pseudonym = hex::encode(thread.creator.0);
        dto.forum_tag.clone_from(&thread.forum_tag);
        dto.auto_archive_seconds = u32::try_from(thread.auto_archive_seconds).unwrap_or(u32::MAX);

        let (last_lamport, last_activity, message_count) =
            thread_activity(deps, thread.record_key.as_deref()).await?;
        dto.last_message_at = last_activity;
        dto.message_count = message_count;
        dto.archived = is_thread_archived(
            thread.archived_lamport,
            last_lamport,
            last_activity,
            thread.auto_archive_seconds,
            now,
        );
        out.push(dto);
    }

    out.sort_by(|a, b| {
        b.last_message_at
            .cmp(&a.last_message_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Phase 19.e — list only non-archived threads.
pub async fn list_active_threads<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<ThreadInfoSnapshot>, ChannelError> {
    let threads = list_threads(deps, community_id, channel_id).await?;
    Ok(threads.into_iter().filter(|t| !t.archived).collect())
}
