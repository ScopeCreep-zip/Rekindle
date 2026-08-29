//! Channel record writes: record creation and per-member entry appends.

use super::codec::{decode_channel_entries, encode_page_entries};
use super::types::{
    ChannelAttachmentCached, ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollClose,
    ChannelPollCreate, ChannelPollVote, ChannelReaction, ChannelRecordEntry,
};
use super::{CHANNEL_MEMBER_SUBKEY_COUNT, CHANNEL_OWNER_SUBKEY_COUNT};
use crate::dht::DHTManager;
use crate::error::ProtocolError;

/// Maximum serialized size for a member's message page (~30KB, leaving DHT overhead room).
const MAX_PAGE_SIZE: usize = 30_000;

/// Create a new SMPL channel record with pre-allocated member slots.
///
/// Uses the same slot seed as the member registry so members derive their
/// writer keypair independently via `derive_slot_veilid_keypair(seed, slot_index)`.
/// Returns `(record_key, owner_keypair)`.
pub async fn create_smpl_channel_record(
    dht: &DHTManager,
    slot_seed: &[u8; 32],
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    use crate::dht::community::member_registry;

    let mut members = Vec::with_capacity(member_registry::SLOTS_PER_SEGMENT as usize);
    for i in 0..member_registry::SLOTS_PER_SEGMENT {
        let signing_key = member_registry::derive_slot_keypair(slot_seed, i)?;
        let public_bytes = signing_key.verifying_key().to_bytes();
        members.push(veilid_core::DHTSchemaSMPLMember {
            m_key: veilid_core::BareMemberId::new(&public_bytes),
            m_cnt: CHANNEL_MEMBER_SUBKEY_COUNT,
        });
    }

    let (key, owner_keypair) = dht
        .create_smpl_record(CHANNEL_OWNER_SUBKEY_COUNT, members)
        .await?;

    tracing::debug!(key = %key, "SMPL channel record created");
    Ok((key, owner_keypair))
}

/// Write a message to a member's subkey in the channel SMPL record.
///
/// Reads existing messages from the subkey, appends the new one, and writes back.
/// If the page exceeds MAX_PAGE_SIZE, oldest messages are dropped from DHT
/// (they're still in SQLite locally).
pub async fn write_member_message(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    message: &ChannelMessage,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::Message(message.clone()),
    )
    .await
}

/// Write a reaction entry to a member's subkey in the channel SMPL record.
pub async fn write_member_reaction(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    reaction: &ChannelReaction,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::Reaction(reaction.clone()),
    )
    .await
}

/// Write a poll create entry to a member's subkey in the channel SMPL record.
pub async fn write_member_poll_create(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_create: &ChannelPollCreate,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::PollCreate(poll_create.clone()),
    )
    .await
}

/// Write a poll vote entry to a member's subkey in the channel SMPL record.
pub async fn write_member_poll_vote(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_vote: &ChannelPollVote,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::PollVote(poll_vote.clone()),
    )
    .await
}

/// Write a poll close entry to a member's subkey in the channel SMPL record.
pub async fn write_member_poll_close(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_close: &ChannelPollClose,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::PollClose(poll_close.clone()),
    )
    .await
}

/// Write a hand-raise entry to a member's subkey in the channel SMPL record.
pub async fn write_member_hand_raise(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    hand_raise: &ChannelHandRaise,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::HandRaise(hand_raise.clone()),
    )
    .await
}

/// Write an `AttachmentCached` entry to a member's subkey in the channel SMPL
/// record (Lost Cargo source advertisement, architecture §28.9 line 3268).
pub async fn write_member_attachment_cached(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    cached: &ChannelAttachmentCached,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::AttachmentCached(cached.clone()),
    )
    .await
}

/// Write a forwarded-message entry to a member's subkey in the channel SMPL record.
///
/// Forwards are stored as their own variant (not wrapped in `Message`) so the
/// destination channel's UI can render the "Forwarded from" attribution
/// without re-parsing message bodies.
pub async fn write_member_forward(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    forward: &ChannelForward,
) -> Result<(), ProtocolError> {
    write_member_entry(
        dht,
        channel_key,
        member_index,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        ChannelRecordEntry::Forward(forward.clone()),
    )
    .await
}

async fn write_member_entry(
    dht: &DHTManager,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    entry: ChannelRecordEntry,
) -> Result<(), ProtocolError> {
    let subkey = u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + member_index;

    // Architecture §26 W26 — only inherit existing entries that pass the
    // author signature check. Otherwise a forger who overwrote our
    // subkey via the shared slot_seed would launder their entries into
    // every subsequent legitimate write we make.
    let mut entries = match dht.get_value(channel_key, subkey).await? {
        Some(data) => decode_channel_entries(&data).unwrap_or_default(),
        None => Vec::new(),
    };

    entries.push(entry);

    let mut bytes = encode_page_entries(
        author_pseudonym.clone(),
        pseudonym_signing_key,
        entries.clone(),
    )?;
    while bytes.len() > MAX_PAGE_SIZE && entries.len() > 1 {
        entries.remove(0);
        bytes = encode_page_entries(
            author_pseudonym.clone(),
            pseudonym_signing_key,
            entries.clone(),
        )?;
    }

    dht.set_value_with_writer(channel_key, subkey, bytes, writer)
        .await
}
