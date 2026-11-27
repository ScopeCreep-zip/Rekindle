//! Prepare a forum thread and its starter for a single durable write.

use rekindle_types::id::ChannelId;

use super::lifecycle::{prepare_thread, ThreadCreation};
use super::messages::{new_reply_id, prepare_reply, PreparedThreadReply, ThreadReplyDraft};
use super::support::hex_to_id_16;
use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

/// A forum post to open.
pub struct ForumPost<'a> {
    pub channel_id: &'a str,
    pub title: &'a str,
    pub forum_tag: Option<String>,
    pub body: &'a str,
}

/// Refuse a post the forum does not take: the channel is not a forum, the
/// tag is not one of its tags, or the title or body is empty.
///
/// # Errors
/// The refusal; the channel or the governance state is unknown.
fn check_forum_post<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    post: &ForumPost<'_>,
) -> Result<(), ChannelError> {
    let gov = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    let channel = gov
        .channels
        .get(&ChannelId(hex_to_id_16(post.channel_id)))
        .ok_or_else(|| ChannelError::ChannelNotFound(post.channel_id.into()))?;
    let refused = |why: &str| Err(ChannelError::InvalidForumPost(why.into()));
    if channel.channel_type != "forum" {
        return refused("the channel is not a forum");
    }
    if let Some(tag) = &post.forum_tag {
        let offered = channel
            .forum_tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|candidate| candidate == tag));
        if !offered {
            return refused("the tag is not one of the forum's tags");
        }
    }
    if post.title.trim().is_empty() {
        return refused("the title is empty");
    }
    if post.body.trim().is_empty() {
        return refused("the starter is empty");
    }
    Ok(())
}

/// Prepare `post` for an atomic governance and outbox commit.
///
/// # Errors
/// The forum refuses the post or we hold no pseudonym.
pub fn prepare_forum_post<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    post: ForumPost<'_>,
) -> Result<
    (
        ThreadCreation,
        rekindle_types::message::ChannelPost,
        PreparedThreadReply,
    ),
    ChannelError,
> {
    check_forum_post(deps, community_id, &post)?;
    let message_id = new_reply_id();
    let thread = prepare_thread(
        deps,
        community_id,
        post.channel_id,
        post.title.trim(),
        &message_id,
        post.forum_tag,
        None,
    )?;
    let (starter, prepared) = prepare_reply(
        deps,
        community_id,
        ThreadReplyDraft {
            thread_id: &thread.row.id,
            parent_channel: post.channel_id,
            revives: None,
            message_id,
            body: post.body,
        },
    )?;
    Ok((thread, starter, prepared))
}
