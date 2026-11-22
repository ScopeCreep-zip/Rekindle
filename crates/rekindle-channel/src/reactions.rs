//! Channel reactions (plan E3.3, rules R1–R4): a reaction is an op on its
//! reactor's channel outbox, aggregated by readers
//! ([`crate::history::build_reaction_groups`]), with the gossip envelope
//! as the fast path.

use rekindle_codec::community::channel_record::ChannelReaction;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};

use rekindle_types::message::{ChannelPost, PostEntry};

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;
use crate::history::MAX_REACTION_KEY_LEN;

/// Pure constructor for the wire `ChannelReaction` payload; the drain
/// passes the entry's stream seq as `lamport`.
#[must_use]
pub fn build_reaction(
    message_id: String,
    expression: String,
    added: bool,
    lamport: u64,
) -> ChannelReaction {
    ChannelReaction {
        message_id,
        expression,
        added,
        lamport,
    }
}

/// Build the `CommunityEnvelope::Control(ReactionAdded|ReactionRemoved)`
/// for gossip broadcast. The `added` flag selects the variant.
#[must_use]
pub fn build_reaction_envelope(
    channel_id: String,
    message_id: String,
    expression: String,
    reactor_pseudonym: String,
    added: bool,
) -> CommunityEnvelope {
    let payload = if added {
        ControlPayload::ReactionAdded {
            channel_id,
            message_id,
            emoji: expression,
            reactor_pseudonym,
        }
    } else {
        ControlPayload::ReactionRemoved {
            channel_id,
            message_id,
            emoji: expression,
            reactor_pseudonym,
        }
    };
    CommunityEnvelope::control(payload)
}

/// Add or remove our reaction `expression` on `message_id` (rules R1–R4):
/// an op on our channel outbox (`M`/4), queued on the channel's
/// `channel_post` stream and drained with our other entries; readers
/// aggregate it from our outbox. The `ReactionAdded`/`ReactionRemoved`
/// gossip is the fast path for online peers.
///
/// # Errors
/// The channel is unknown, the key is empty or over
/// [`MAX_REACTION_KEY_LEN`], we hold no pseudonym, or the queue failed.
pub async fn persist_reaction<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    expression: &str,
    added: bool,
) -> Result<(), ChannelError> {
    if expression.is_empty() || expression.len() > MAX_REACTION_KEY_LEN {
        return Err(ChannelError::Adapter(format!(
            "a reaction key is 1 to {MAX_REACTION_KEY_LEN} bytes"
        )));
    }
    deps.channel_info(community_id, channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    let reactor_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.to_string()))?;
    let post = ChannelPost {
        community: community_id.to_owned(),
        channel: channel_id.to_owned(),
        message: format!(
            "reaction:{}",
            hex::encode(rekindle_utils::random::id_bytes_16())
        ),
        entry: PostEntry::Reaction {
            target: message_id.to_owned(),
            emoji: expression.to_owned(),
            added,
        },
    };
    deps.queue_post(&post).await?;
    let envelope = build_reaction_envelope(
        channel_id.to_string(),
        message_id.to_string(),
        expression.to_string(),
        reactor_pseudonym,
        added,
    );
    if let Err(error) = deps.send_to_mesh(community_id, &envelope) {
        tracing::debug!(community_id, %error, "reaction gossip not sent; readers take it from our outbox");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_reaction_carries_fields_verbatim() {
        let r = build_reaction("msg_1".into(), "👍".into(), true, 42);
        assert_eq!(r.message_id, "msg_1");
        assert_eq!(r.expression, "👍");
        assert!(r.added);
        assert_eq!(r.lamport, 42);

        let r_remove = build_reaction("msg_2".into(), "❤️".into(), false, 99);
        assert!(!r_remove.added);
        assert_eq!(r_remove.lamport, 99);
    }

    #[test]
    fn build_envelope_added_branch_is_reaction_added() {
        let env = build_reaction_envelope(
            "chan".into(),
            "msg".into(),
            "✨".into(),
            "pseu".into(),
            true,
        );
        match env.into_control() {
            Some(ControlPayload::ReactionAdded {
                channel_id,
                message_id,
                emoji,
                reactor_pseudonym,
            }) => {
                assert_eq!(channel_id, "chan");
                assert_eq!(message_id, "msg");
                assert_eq!(emoji, "✨");
                assert_eq!(reactor_pseudonym, "pseu");
            }
            other => panic!("expected ReactionAdded, got {other:?}"),
        }
    }

    #[test]
    fn build_envelope_removed_branch_is_reaction_removed() {
        let env = build_reaction_envelope(
            "chan".into(),
            "msg".into(),
            "🔥".into(),
            "pseu".into(),
            false,
        );
        assert!(matches!(
            env.as_control(),
            Some(ControlPayload::ReactionRemoved { .. })
        ));
    }
}
