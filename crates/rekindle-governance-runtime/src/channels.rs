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

/// What the caller needs back to update its own local view.
#[derive(Debug, Clone)]
pub struct CreatedChannel {
    pub channel_id: ChannelId,
    pub channel_id_hex: String,
    pub record_key: String,
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

/// Mint a channel's SMPL record and announce it.
///
/// The record uses the universal schema keyed by the community's shared
/// slot seed, so every member can derive their own writer keypair for it
/// without being handed anything.
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

    let member_pubkeys = slot_pubkeys(&slot_seed)?;
    // Taken before the create, so nothing can fail between the create and
    // the announcement that hands the record's lease to the host.
    let record = deps.create_smpl_record(&member_pubkeys).await?;
    if let Err(e) = crate::records::publish_created(deps, record.lease, &slot_seed).await {
        deps.release_record(record.lease).await;
        return Err(e);
    }

    // Random, not derived from the name: two members creating a
    // same-named channel concurrently must get two channels, not one
    // colliding id whose `ChannelCreated` entries LWW each other and
    // strand one record.
    let channel_id = ChannelId(rekindle_utils::random::id_bytes_16_from(
        deps.rng().as_ref(),
    ));
    let channel_id_hex = hex::encode(channel_id.0);

    let announced = apply::write_entry(
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
        },
    )
    .await;
    if let Err(e) = announced {
        deps.release_record(record.lease).await;
        return Err(e);
    }
    // The community keeps the new channel's record for its session.
    deps.community_records_ready(
        community_id,
        rekindle_records::lease::CommunityLeases {
            channels: std::collections::HashMap::from([(channel_id, record.lease)]),
            ..Default::default()
        },
    )
    .await;

    Ok(CreatedChannel {
        channel_id,
        channel_id_hex,
        record_key: record.record_key,
        position: channel.position,
    })
}

/// The 255 slot public keys that define a channel record's SMPL schema:
/// one per slot of the community's registry, derived from the shared slot
/// seed, so any member derives their own writer credential for a record
/// they have only just heard about.
pub(crate) fn slot_pubkeys(slot_seed: &[u8; 32]) -> Result<Vec<[u8; 32]>, GovernanceRuntimeError> {
    let slots = rekindle_types::dht_layout::REGISTRY_SLOTS;
    let mut member_pubkeys = Vec::with_capacity(slots as usize);
    for slot in 0..slots {
        let sk = rekindle_secrets::derive::derive_slot_keypair(slot_seed, slot).map_err(|e| {
            GovernanceRuntimeError::Crypto(format!("derive_slot_keypair {slot}: {e}"))
        })?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }
    Ok(member_pubkeys)
}
