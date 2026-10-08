//! Thread-creation permission checks (public / private / announcement /
//! forum-post), gated on channel-scoped permissions.

use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{CREATE_PRIVATE_THREADS, MANAGE_THREADS, SEND_MESSAGES};

use super::has;
use crate::permissions::compute_permissions;
use crate::state::GovernanceState;

pub(super) fn validate_thread_create(
    writer: &PseudonymKey,
    parent_channel_id: rekindle_types::id::ChannelId,
    thread_type: &str,
    invited: &[PseudonymKey],
    forum_tag: Option<&str>,
    state: &GovernanceState,
) -> bool {
    let channel_perms = compute_permissions(
        writer,
        Some(&parent_channel_id),
        state,
        rekindle_utils::time::timestamp_secs(),
    );

    match thread_type {
        "public" => has(channel_perms, SEND_MESSAGES),
        "private" => {
            has(channel_perms, CREATE_PRIVATE_THREADS)
                && invited.iter().all(|invitee| invitee != writer)
        }
        "announcement" => has(channel_perms, MANAGE_THREADS),
        "forum_post" => {
            state
                .channels
                .get(&parent_channel_id)
                .is_some_and(|channel| {
                    channel.channel_type == "forum"
                        && forum_tag.is_none_or(|tag| {
                            channel
                                .forum_tags
                                .as_ref()
                                .is_some_and(|tags| tags.iter().any(|candidate| candidate == tag))
                        })
                })
                && has(channel_perms, SEND_MESSAGES)
        }
        _ => false,
    }
}
