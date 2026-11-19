//! Creating a channel, once, for both tracks: a `ChannelCreated`
//! governance op. A channel has no record of its own: each member's posts
//! ride that member's `M`/4 outbox under the channel's tag (plan E3.3,
//! ADR 0011), so there is nothing to mint or announce besides the op.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{CategoryId, ChannelId};

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// What the caller needs back to update its own local view.
#[derive(Debug, Clone)]
pub struct CreatedChannel {
    pub channel_id: ChannelId,
    pub channel_id_hex: String,
    pub position: u32,
}

/// The fields a caller chooses; everything else is derived.
#[derive(Debug, Clone)]
pub struct NewChannel {
    pub name: String,
    /// "text", "voice", "announcement", "forum", "stage", "media"
    pub channel_type: String,
    pub category_id: Option<CategoryId>,
    pub position: u32,
    pub parent_voice_channel_id: Option<ChannelId>,
}

/// Announce a new channel: its `ChannelCreated` op.
pub async fn create_channel<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    channel: NewChannel,
) -> Result<CreatedChannel, GovernanceRuntimeError> {
    if deps.community_membership(community_id).is_none() {
        return Err(GovernanceRuntimeError::CommunityNotFound(
            community_id.to_string(),
        ));
    }
    // Random, not derived from the name: two members creating a
    // same-named channel concurrently must get two channels, not one
    // colliding id whose `ChannelCreated` entries LWW each other and
    // strand one record.
    let channel_id = ChannelId(rekindle_utils::random::id_bytes_16_from(
        deps.rng().as_ref(),
    ));
    let channel_id_hex = hex::encode(channel_id.0);

    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::ChannelCreated {
            channel_id,
            name: channel.name,
            channel_type: channel.channel_type,
            category_id: channel.category_id,
            position: channel.position,
            parent_voice_channel_id: channel.parent_voice_channel_id,
        },
    )
    .await?;

    Ok(CreatedChannel {
        channel_id,
        channel_id_hex,
        position: channel.position,
    })
}
