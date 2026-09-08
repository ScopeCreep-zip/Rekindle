//! Creating a channel, once, for both tracks.
//!
//! A channel is two things landing together: an SMPL record for its
//! messages, and a `ChannelCreated` governance entry naming that record.
//! Neither is useful alone — an unannounced record is undiscoverable,
//! and an entry without a record points nowhere — so they are minted in
//! one place rather than assembled per shell.
//!
//! The daemon previously did neither. `channel_admin::create_channel`
//! appended a `ChannelEntry` to the governance manifest's v1.0 channels
//! subkey with `message_record_key: None`, so a daemon-created channel
//! was invisible to the CRDT every peer actually merges and had nowhere
//! to write messages to.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{CategoryId, ChannelId};

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::segments::segment_slot_pubkeys;

/// What the caller needs back to update its own local view.
#[derive(Debug, Clone)]
pub struct CreatedChannel {
    pub channel_id: ChannelId,
    pub channel_id_hex: String,
    pub record_key: String,
    pub position: u32,
    pub lamport: u64,
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

/// Mint a channel's segment-0 SMPL record and announce it.
///
/// The record uses the universal schema keyed by the community's shared
/// slot seed, so every member can derive their own writer keypair for it
/// without being handed anything. Members in segment N > 0 create their
/// own record lazily on first write — see
/// [`crate::segments::ensure_channel_segment_record`].
pub async fn create_channel<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    channel: NewChannel,
) -> Result<CreatedChannel, GovernanceRuntimeError> {
    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;
    let slot_seed_hex = membership
        .slot_seed_hex
        .ok_or_else(|| GovernanceRuntimeError::SlotSeedMissing(community_id.to_string()))?;
    let slot_seed: [u8; 32] = hex::decode(&slot_seed_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| {
            GovernanceRuntimeError::Crypto("slot seed is not 32 hex-encoded bytes".to_string())
        })?;

    // Segment 0. Later segments derive from global slot indices, which
    // is why this goes through the same helper rather than hardcoding
    // `0..255`.
    let member_pubkeys = segment_slot_pubkeys(&slot_seed, 0)?;
    let record = deps.create_smpl_record(&member_pubkeys).await?;

    // Random, not derived from the name: two members creating a
    // same-named channel concurrently must get two channels, not one
    // colliding id whose `ChannelCreated` entries LWW each other and
    // strand one record.
    let channel_id = ChannelId(rekindle_utils::random::id_bytes_16());
    let channel_id_hex = hex::encode(channel_id.0);
    let lamport = deps.increment_lamport(community_id);

    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::ChannelCreated {
            channel_id,
            name: channel.name,
            channel_type: channel.channel_type,
            record_key: record.record_key.clone(),
            category_id: channel.category_id,
            position: channel.position,
            parent_voice_channel_id: channel.parent_voice_channel_id,
            lamport,
        },
    )
    .await?;

    Ok(CreatedChannel {
        channel_id,
        channel_id_hex,
        record_key: record.record_key,
        position: channel.position,
        lamport,
    })
}
