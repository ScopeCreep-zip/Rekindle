//! Phase 19.c-REDO — full channel-reaction pipeline.
//!
//! Pure helpers (build_reaction / build_reaction_envelope) plus the
//! orchestrator `persist_reaction<D: ChannelMessagingDeps>` ported
//! verbatim from src-tauri/services/community/channel_reactions.rs.
//! Same shape as Phase 18 apply::write_entry — full body in crate
//! parameterised over the deps trait; src-tauri retains a thin facade
//! after 19.h-REDO wires the adapter.

use rekindle_protocol::dht::community::channel_record::ChannelReaction;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

/// Pure constructor for the wire `ChannelReaction` payload. The
/// `lamport` value is supplied by the orchestrator (already
/// incremented). All fields are owned strings/values — adapter can
/// hand the result to `write_member_reaction` without further
/// transformation.
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
    CommunityEnvelope::Control(payload)
}

/// Phase 19.c-REDO — full persist_reaction pipeline.
///
/// Writes a reaction to the channel's SMPL record AND broadcasts the
/// `ReactionAdded`/`ReactionRemoved` envelope to the mesh. Best-effort
/// dual-path: either the SMPL write OR the gossip path succeeding is
/// considered overall success (architecture §28.6 — receivers update
/// from whichever arrives first; the SMPL record is authoritative for
/// late joiners; the gossip path is the fast-path for online peers).
pub async fn persist_reaction<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    expression: &str,
    added: bool,
) -> Result<(), ChannelError> {
    let context = deps.channel_write_context(community_id, channel_id)?;
    let reactor_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.to_string()))?;
    let lamport = deps.increment_lamport(community_id);
    let reaction = build_reaction(
        message_id.to_string(),
        expression.to_string(),
        added,
        lamport,
    );

    let write_result = deps.write_member_reaction_smpl(&context, &reaction).await;
    let envelope = build_reaction_envelope(
        channel_id.to_string(),
        message_id.to_string(),
        expression.to_string(),
        reactor_pseudonym,
        added,
    );
    let gossip_result = deps.send_to_mesh(community_id, &envelope);

    match (write_result, gossip_result) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(write_error), Err(gossip_error)) => Err(ChannelError::Adapter(format!(
            "reaction delivery failed: SMPL write: {write_error}; gossip notify: {gossip_error}"
        ))),
    }
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
        match env {
            CommunityEnvelope::Control(ControlPayload::ReactionAdded {
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
            env,
            CommunityEnvelope::Control(ControlPayload::ReactionRemoved { .. })
        ));
    }
}
