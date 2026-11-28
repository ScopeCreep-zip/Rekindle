//! A channel's reactions and polls (plan E3.3): row-less ops on their
//! authors' channel outboxes (`M`/4), aggregated by every reader and
//! attached by the host to the rows its outbox reader stored.

mod materialize;
mod poll_states;

use std::collections::HashMap;

use rekindle_types::dto::channel_message::ReactionGroup;

use crate::deps::ChannelMessagingDeps;

pub use materialize::{build_reaction_groups, MAX_REACTIONS_PER_MESSAGE, MAX_REACTION_KEY_LEN};
pub use poll_states::{build_poll_states, poll_tallies, PollTally, PollsByMessage};

/// A channel's reactions by message id and its polls by (message id,
/// creator), as its authors' outboxes leave them.
#[derive(Debug, Default)]
pub struct ChannelAttachments {
    pub reactions: HashMap<String, Vec<ReactionGroup>>,
    pub polls: PollsByMessage,
}

/// `channel_id`'s reactions and polls, read from its authors' outboxes.
pub async fn read_attachments<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> ChannelAttachments {
    let outbox = deps.read_outbox_entries(community_id, channel_id).await;
    let me = deps.my_pseudonym_hex(community_id).unwrap_or_default();
    ChannelAttachments {
        reactions: build_reaction_groups(&outbox),
        polls: build_poll_states(&outbox, &me),
    }
}
