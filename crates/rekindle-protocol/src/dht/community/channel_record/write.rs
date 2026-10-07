//! Channel record writes: record creation and per-member entry appends.

use super::codec::{decode_own_page, encode_page_entries};
use super::types::{
    ChannelAttachmentCached, ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollClose,
    ChannelPollCreate, ChannelPollVote, ChannelReaction, ChannelRecordEntry,
};
use super::{CHANNEL_MEMBER_SUBKEY_COUNT, CHANNEL_OWNER_SUBKEY_COUNT};
use crate::dht::parse_record_key;
use crate::dht::pool::{RecordPool, SetOutcome};
use crate::error::ProtocolError;

/// Maximum serialized size for a member's message page (~30KB, leaving DHT overhead room).
const MAX_PAGE_SIZE: usize = 30_000;

/// Create a new SMPL channel record with pre-allocated member slots.
///
/// Uses the same slot seed as the member registry so members derive their
/// writer keypair independently via `derive_slot_veilid_keypair(seed, slot_index)`.
/// Returns `(creator lease, record_key, owner_keypair)`.
pub async fn create_smpl_channel_record(
    pool: &RecordPool,
    slot_seed: &[u8; 32],
) -> Result<
    (
        rekindle_records::lease::LeaseId,
        String,
        veilid_core::KeyPair,
    ),
    ProtocolError,
> {
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

    let schema = veilid_core::DHTSchema::smpl(CHANNEL_OWNER_SUBKEY_COUNT, members)
        .map_err(|e| ProtocolError::DhtError(format!("invalid channel schema: {e}")))?;
    // The creator's lease is returned for the caller to hand to its
    // community, so the record stays open for the watch and writes that
    // follow.
    let (lease, key, owner_keypair) = pool.create(schema, None).await?;
    // A create is local only; the network learns the record from its first
    // set (`storage_manager/create_record.rs`). Publish it now with an empty
    // value in the last slot, under that slot's writer from the shared seed:
    // readers treat an empty page as no entries, and a joiner takes that
    // slot last (plan C7.6d).
    let last = member_registry::SLOTS_PER_SEGMENT - 1;
    let writer = member_registry::derive_slot_veilid_keypair(slot_seed, last)?;
    let published = pool
        .set(
            lease,
            u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + last,
            Vec::new(),
            Some(writer),
        )
        .await
        .and_then(|outcome| outcome.require_stored(last));
    if let Err(e) = published {
        pool.release(lease).await;
        return Err(e);
    }
    let key = key.to_string();
    tracing::debug!(key = %key, "SMPL channel record created and published");
    Ok((lease, key, owner_keypair))
}

/// Write a message to a member's subkey in the channel SMPL record.
///
/// Reads existing messages from the subkey, appends the new one, and writes back.
/// If the page exceeds MAX_PAGE_SIZE, oldest messages are dropped from DHT
/// (they're still in SQLite locally).
pub async fn write_member_message(
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    message: &ChannelMessage,
) -> Result<AppendOutcome, ProtocolError> {
    write_member_entry(
        pool,
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
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    reaction: &ChannelReaction,
) -> Result<AppendOutcome, ProtocolError> {
    write_member_entry(
        pool,
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
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_create: &ChannelPollCreate,
) -> Result<AppendOutcome, ProtocolError> {
    write_member_entry(
        pool,
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
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_vote: &ChannelPollVote,
) -> Result<AppendOutcome, ProtocolError> {
    write_member_entry(
        pool,
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
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    poll_close: &ChannelPollClose,
) -> Result<AppendOutcome, ProtocolError> {
    write_member_entry(
        pool,
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
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    hand_raise: &ChannelHandRaise,
) -> Result<AppendOutcome, ProtocolError> {
    write_member_entry(
        pool,
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
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    cached: &ChannelAttachmentCached,
) -> Result<AppendOutcome, ProtocolError> {
    write_member_entry(
        pool,
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
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    forward: &ChannelForward,
) -> Result<AppendOutcome, ProtocolError> {
    write_member_entry(
        pool,
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
    pool: &RecordPool,
    channel_key: &str,
    member_index: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    entry: ChannelRecordEntry,
) -> Result<AppendOutcome, ProtocolError> {
    let subkey = u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + member_index;

    // Architecture §26 W26 — only inherit existing entries that pass the
    // author signature check. Otherwise a forger who overwrote our
    // subkey via the shared slot_seed would launder their entries into
    // every subsequent legitimate write we make.
    // Borrowed writable with our slot writer, so a write restored after a
    // re-login (which keeps no writer) re-pushes as us (plan C7.13).
    let lease = pool
        .acquire(&parse_record_key(channel_key)?, Some(writer.clone()))
        .await?;
    let written = append_entry(
        pool,
        lease,
        subkey,
        writer,
        author_pseudonym,
        pseudonym_signing_key,
        entry,
    )
    .await;
    pool.release(lease).await;
    written
}

/// Bound on compare-and-swap rounds when the network keeps holding a newer
/// page of our slot.
const CAS_ROUNDS: usize = 3;

/// Whether `entry` is already among `entries` (same canonical encoding):
/// an append is idempotent, and a merge keeps one copy of each entry.
pub(super) fn contains_entry(entries: &[ChannelRecordEntry], entry: &ChannelRecordEntry) -> bool {
    let Ok(wanted) = serde_json::to_vec(entry) else {
        return false;
    };
    entries
        .iter()
        .any(|have| serde_json::to_vec(have).is_ok_and(|bytes| bytes == wanted))
}

/// `theirs` (the network's newer page) with every entry of `ours` it lacks,
/// in lamport order: the compare-and-swap merge of an append.
pub(super) fn merge_entries(
    theirs: Vec<ChannelRecordEntry>,
    ours: Vec<ChannelRecordEntry>,
) -> Vec<ChannelRecordEntry> {
    let mut merged = theirs;
    for entry in ours {
        if !contains_entry(&merged, &entry) {
            merged.push(entry);
        }
    }
    merged.sort_by_key(ChannelRecordEntry::lamport);
    merged
}

/// Encode `entries` signed by the author, dropping the oldest until the page
/// fits `MAX_PAGE_SIZE` (they stay in SQLite locally).
fn encode_trimmed(
    author_pseudonym: &rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    entries: &mut Vec<ChannelRecordEntry>,
) -> Result<Vec<u8>, ProtocolError> {
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
    Ok(bytes)
}

/// What became of an append on the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    /// The page is stored at consensus.
    Stored,
    /// The page missed consensus; the record pool holds it, re-pushes it
    /// until it lands, and publishes the settle (`RecordPool::settled`).
    Held,
}

/// Read our page, append `entry` (once: a re-run finds it already there),
/// trim to `MAX_PAGE_SIZE`, and write it back as a durable write with the
/// member's slot writer (plan C7.13). The record pool is the one retry
/// layer: a miss is held and re-pushed, not re-appended. `Superseded` (the
/// network holds a newer page of our slot) is a compare-and-swap conflict:
/// merge the newer page, if it is ours, and write again.
///
/// # Errors
/// A write that failed outright, or a slot still superseded after
/// `CAS_ROUNDS` merges (`NotStored`).
async fn append_entry(
    pool: &RecordPool,
    lease: rekindle_records::lease::LeaseId,
    subkey: u32,
    writer: veilid_core::KeyPair,
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    entry: ChannelRecordEntry,
) -> Result<AppendOutcome, ProtocolError> {
    // Architecture §26 W26 — only inherit a page we wrote. A page another
    // key wrote into our slot (anyone holding the shared slot_seed can) is
    // neither re-signed as ours nor kept.
    let mut entries = match pool.get(lease, subkey, false).await? {
        Some(data) => decode_own_page(data.data(), &author_pseudonym).unwrap_or_default(),
        None => Vec::new(),
    };
    if !contains_entry(&entries, &entry) {
        entries.push(entry);
    }

    for _ in 0..CAS_ROUNDS {
        let bytes = encode_trimmed(&author_pseudonym, pseudonym_signing_key, &mut entries)?;
        match pool
            .set_durable_as(lease, subkey, bytes, Some(writer.clone()))
            .await?
        {
            SetOutcome::Landed | SetOutcome::Unchanged => return Ok(AppendOutcome::Stored),
            SetOutcome::BelowConsensus | SetOutcome::Offline => return Ok(AppendOutcome::Held),
            SetOutcome::Superseded(newer) => {
                let theirs = decode_own_page(newer.data(), &author_pseudonym).unwrap_or_default();
                entries = merge_entries(theirs, entries);
            }
        }
    }
    Err(ProtocolError::NotStored {
        subkey,
        outcome: format!("still superseded after {CAS_ROUNDS} merges"),
    })
}
