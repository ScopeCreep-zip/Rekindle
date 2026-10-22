//! The user's Retry of a failed channel post (plan E2.5): its write is
//! made again under the **same** message id, through the send path that
//! made it. The post's outbox row was queued again by the outbox's Retry;
//! the write's outcome settles it like a first send's (a stored page
//! `sent`, a held one by the pool's settle, one that cannot be made
//! `failed`).

use rekindle_types::message::SendState;

use crate::deps::{ChannelMessagingDeps, ChannelPost, DhtWrite};
use crate::error::ChannelError;
use crate::mentions::resolve_outbound_mentions;
use crate::pipeline::{build_message_notification, emit_settled};
use crate::send::{
    build_channel_message, channel_message_subkey, BodyPosition, BuildChannelMessageParams,
};

/// Write `post` again: its stored body sealed afresh at the slot's next
/// position, under its message id. Returns the post's state.
///
/// # Errors
/// The post is not ours in the local history, or the channel's write slot
/// is no longer the one its row waits on (Discard it and send again); its
/// row is untouched, and the outbox fails it with the reason.
pub async fn reissue_channel_post<D: ChannelMessagingDeps>(
    deps: &D,
    post: &ChannelPost,
) -> Result<SendState, ChannelError> {
    let (community_id, channel_id) = (post.community.as_str(), post.channel.as_str());
    let stored = deps
        .find_channel_message_by_id(channel_id, &post.message)
        .await
        .ok_or_else(|| ChannelError::Adapter(format!("post {} not in history", post.message)))?;
    let sender_key = deps
        .my_pseudonym_hex(community_id)
        .or_else(|| deps.owner_key())
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    if stored.sender_key != sender_key {
        return Err(ChannelError::Adapter(format!(
            "post {} is not ours",
            post.message
        )));
    }
    let context = deps.channel_write_context(community_id, channel_id)?;
    let subkey = channel_message_subkey(context.slot_index);
    if context.channel_key != post.record_key || subkey != post.subkey {
        return Err(ChannelError::Adapter(
            "the channel's write slot changed".into(),
        ));
    }
    let lamport_ts = deps.increment_lamport(community_id)?;
    let (ciphertext, mek_generation) = crate::text_keys::seal_text(
        deps,
        community_id,
        channel_id,
        BodyPosition {
            channel_record_key: &context.channel_key,
            subkey_index: subkey,
            lamport_ts,
        },
        stored.body.as_bytes(),
    )?;
    let sequence = deps.next_channel_sequence(community_id, channel_id);
    deps.persist_channel_sequence(community_id, channel_id, sequence)
        .await?;
    let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
        resolve_outbound_mentions(deps, community_id, &sender_key, &stored.body);
    let timestamp_ms = i64::try_from(deps.now_secs() * 1000).unwrap_or(i64::MAX);
    let channel_msg = build_channel_message(BuildChannelMessageParams {
        sequence,
        sender_pseudonym: sender_key,
        ciphertext,
        mek_generation,
        timestamp_ms,
        lamport_ts,
        message_id: post.message.clone(),
        reply_to: stored.reply_to.clone(),
        mention_flag_bits: mention_flags,
        mentioned_pseudonyms,
        mentioned_roles,
    });
    let written = deps
        .write_channel_message_smpl(&context, &channel_msg)
        .await;
    let state = match written {
        Ok(written) => {
            let notification =
                build_message_notification(channel_id, &channel_msg, context.slot_index)?;
            let _ = deps.send_to_mesh(community_id, &notification);
            if written == DhtWrite::Stored {
                let settled = deps.settle_posts(&post.record_key, post.subkey, true).await;
                emit_settled(deps, &settled, true, None);
                SendState::Sent
            } else {
                SendState::Queued
            }
        }
        Err(error) => {
            tracing::warn!(%error, message = %post.message, "channel post rewrite failed");
            deps.fail_post(&post.message).await;
            deps.emit_delivery_failed(community_id, channel_id, &post.message);
            SendState::Failed
        }
    };
    Ok(state)
}
