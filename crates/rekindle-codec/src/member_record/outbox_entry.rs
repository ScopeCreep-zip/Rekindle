//! A channel outbox entry (plan E3.3, `M`/4): one entry of a member's
//! channel outbox chain, the N2 chain whose head `M`/4 holds. The entry's
//! chain position is its N2 seq, never stored; its signature binds it to
//! the community anchor and that seq, so an entry copied to another chain
//! position or community does not verify.
//!
//! `prev` hash-links the author's whole outbox (Secure Scuttlebutt
//! `previous`, Matrix `prev_events`) over the bytes as stored, so a reader
//! that cannot open a restricted entry still checks continuity.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::domains;

use super::{put_lp, sign, verify};
use crate::capnp_codec::{bounds, capnp_err, pack, unpack_bounded};
use crate::community::channel_record::ChannelRecordEntry;
use crate::member_record_capnp::{outbox_entry, OutboxEntryKind};
use crate::CodecError;

/// A channel tag's length (`rekindle_secrets::channel_body::CHANNEL_TAG_LEN`).
pub const CHANNEL_TAG_LEN: usize = rekindle_secrets::channel_body::CHANNEL_TAG_LEN;

/// One entry of an author's channel outbox, unsigned.
#[derive(Debug, Clone)]
pub struct OutboxEntry {
    /// BLAKE3 of the previous entry's bytes in the author's chain; `None`
    /// for the chain's first entry.
    pub prev: Option<[u8; 32]>,
    pub channel_tag: [u8; CHANNEL_TAG_LEN],
    /// The per-(author, tag) counter, frozen at creation: the body's AAD
    /// position and the dedup key, never a cursor.
    pub stream_seq: u64,
    /// The text key generation the tag and the body are under.
    pub generation: u64,
    /// The author's clock at commit, in ms.
    pub committed_at: u64,
    pub entry: ChannelRecordEntry,
}

/// BLAKE3 of an entry's stored bytes: the next entry's `prev`.
#[must_use]
pub fn entry_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// Whether `entry` links to the entry before it in its author's chain:
/// `before` is the [`entry_hash`] of that entry's stored bytes, `None`
/// when `entry` is the chain's first. A reader records a broken link as an
/// anomaly and still applies the entry (plan E3.3 "Per-author chain").
#[must_use]
pub fn links_to(entry: &OutboxEntry, before: Option<[u8; 32]>) -> bool {
    entry.prev == before
}

fn kind_of(entry: &ChannelRecordEntry) -> OutboxEntryKind {
    match entry {
        ChannelRecordEntry::Message(_) => OutboxEntryKind::Message,
        ChannelRecordEntry::Forward(_) => OutboxEntryKind::Forward,
        ChannelRecordEntry::AttachmentCached(_) => OutboxEntryKind::AttachmentCached,
        ChannelRecordEntry::Reaction(_) => OutboxEntryKind::Reaction,
        ChannelRecordEntry::PollCreate(_) => OutboxEntryKind::PollCreate,
        ChannelRecordEntry::PollVote(_) => OutboxEntryKind::PollVote,
        ChannelRecordEntry::PollClose(_) => OutboxEntryKind::PollClose,
        ChannelRecordEntry::HandRaise(_) => OutboxEntryKind::HandRaise,
        ChannelRecordEntry::Edit(_) => OutboxEntryKind::Edit,
        ChannelRecordEntry::Delete(_) => OutboxEntryKind::Delete,
    }
}

/// Where an entry sits: the community anchor and its N2 chain seq.
#[derive(Debug, Clone, Copy)]
pub struct ChainPosition<'a> {
    pub anchor: &'a str,
    pub chain_seq: u64,
}

impl OutboxEntry {
    fn signing_bytes(
        &self,
        at: ChainPosition<'_>,
        kind: OutboxEntryKind,
        sealed: &[u8],
    ) -> Vec<u8> {
        let mut out = domains::OUTBOX_ENTRY.as_bytes().to_vec();
        put_lp(&mut out, at.anchor.as_bytes());
        out.extend_from_slice(&at.chain_seq.to_be_bytes());
        put_lp(&mut out, self.prev.as_ref().map_or(&[][..], |p| &p[..]));
        out.extend_from_slice(&self.channel_tag);
        out.extend_from_slice(&self.stream_seq.to_be_bytes());
        out.extend_from_slice(&self.generation.to_be_bytes());
        out.extend_from_slice(&self.committed_at.to_be_bytes());
        out.extend_from_slice(&(kind as u16).to_be_bytes());
        put_lp(&mut out, sealed);
        out
    }

    /// Sign by the author's pseudonym `author` at `at` and encode.
    ///
    /// # Errors
    /// The entry does not encode.
    pub fn encode(
        &self,
        at: ChainPosition<'_>,
        author: &SigningKey,
    ) -> Result<Vec<u8>, CodecError> {
        let sealed = serde_json::to_vec(&self.entry)
            .map_err(|e| CodecError::Serialization(format!("outbox entry: {e}")))?;
        let kind = kind_of(&self.entry);
        let sig = sign(author, &self.signing_bytes(at, kind, &sealed));
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<outbox_entry::Builder<'_>>();
            b.set_prev(self.prev.as_ref().map_or(&[][..], |p| &p[..]));
            b.set_channel_tag(&self.channel_tag);
            b.set_stream_seq(self.stream_seq);
            b.set_generation(self.generation);
            b.set_committed_at(self.committed_at);
            b.set_kind(kind);
            b.set_sealed(&sealed);
            b.set_sig(&sig);
        }
        // The size is the writer's check (`EntryTooLarge`, one sealed page)
        // and the decoder's bound.
        Ok(pack(&builder))
    }
}

/// Decode the entry at `at` of `author`'s chain and verify its signature.
///
/// # Errors
/// Over the bound, malformed, a kind that does not match its entry, or a
/// bad signature.
pub fn decode_outbox_entry(
    data: &[u8],
    at: ChainPosition<'_>,
    author: &[u8; 32],
) -> Result<OutboxEntry, CodecError> {
    let reader = unpack_bounded(data, bounds::OUTBOX_ENTRY_BOUND)?;
    let b = reader
        .get_root::<outbox_entry::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let malformed = |what: &str| CodecError::Deserialization(format!("outbox entry: {what}"));
    let prev = match b.get_prev().map_err(|e| capnp_err(&e))? {
        [] => None,
        p => Some(<[u8; 32]>::try_from(p).map_err(|_| malformed("prev is not 32 bytes"))?),
    };
    let channel_tag = b
        .get_channel_tag()
        .map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| malformed("channel tag is not 16 bytes"))?;
    let kind = b.get_kind().map_err(|_| malformed("unknown kind"))?;
    let sealed = b.get_sealed().map_err(|e| capnp_err(&e))?;
    let entry: ChannelRecordEntry =
        serde_json::from_slice(sealed).map_err(|e| malformed(&e.to_string()))?;
    if kind_of(&entry) != kind {
        return Err(malformed("kind does not match its entry"));
    }
    let decoded = OutboxEntry {
        prev,
        channel_tag,
        stream_seq: b.get_stream_seq(),
        generation: b.get_generation(),
        committed_at: b.get_committed_at(),
        entry,
    };
    verify(
        author,
        &decoded.signing_bytes(at, kind, sealed),
        b.get_sig().map_err(|e| capnp_err(&e))?,
        "outbox entry",
    )?;
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::community::channel_record::ChannelDelete;

    fn entry(prev: Option<[u8; 32]>) -> OutboxEntry {
        OutboxEntry {
            prev,
            channel_tag: [5; 16],
            stream_seq: 9,
            generation: 2,
            committed_at: 1_700_000_000_000,
            entry: ChannelRecordEntry::Delete(ChannelDelete {
                message_id: "msg_1".into(),
                deleted_at: 1,
                lamport_ts: 3,
            }),
        }
    }

    const AT: ChainPosition<'static> = ChainPosition {
        anchor: "VLD0:anchor",
        chain_seq: 4,
    };

    #[test]
    fn an_entry_round_trips_under_its_author_and_position() {
        let author = SigningKey::from_bytes(&[3; 32]);
        let public = author.verifying_key().to_bytes();
        let bytes = entry(Some([1; 32])).encode(AT, &author).unwrap();
        let back = decode_outbox_entry(&bytes, AT, &public).unwrap();
        assert_eq!(back.prev, Some([1; 32]));
        assert_eq!((back.stream_seq, back.generation), (9, 2));
        assert!(matches!(back.entry, ChannelRecordEntry::Delete(_)));
        let first = entry(None).encode(AT, &author).unwrap();
        assert_eq!(decode_outbox_entry(&first, AT, &public).unwrap().prev, None);
    }

    /// Another chain seq, anchor or author does not verify.
    #[test]
    fn an_entry_is_bound_to_its_chain_position_and_author() {
        let author = SigningKey::from_bytes(&[3; 32]);
        let public = author.verifying_key().to_bytes();
        let bytes = entry(None).encode(AT, &author).unwrap();
        let moved = ChainPosition { chain_seq: 5, ..AT };
        assert!(decode_outbox_entry(&bytes, moved, &public).is_err());
        let other = ChainPosition {
            anchor: "VLD0:other",
            ..AT
        };
        assert!(decode_outbox_entry(&bytes, other, &public).is_err());
        let stranger = SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes();
        assert!(decode_outbox_entry(&bytes, AT, &stranger).is_err());
    }
}
