//! The traversal bound of every Cap'n Proto decoder (plan F4, research
//! batch 8 AO §1).
//!
//! Every value these decoders read came off the network: a Veilid
//! `app_message`/`app_call` from a peer, or a DHT subkey any writer of the
//! record could fill. capnp's default traversal limit is 8 Mi words
//! (64 MiB), and packing collapses a run of 256 zero words into two bytes,
//! so without a bound a 32 KiB datagram could unpack to tens of megabytes.
//! [`super::unpack_bounded`] is the only reader, and each decoder passes the
//! [`Bound`] below for its type: a byte cap checked before the input is
//! scanned, and a traversal limit in words.
//!
//! A bound is [`words_for`] the largest packed value its carrier delivers:
//! four times that many bytes, as words. A legitimate message unpacks to
//! about its packed size, so four times leaves room for any value that fits
//! the carrier, while a zero-run bomb is refused. This is the rule the
//! governance op, genesis cert and DM frame bounds already follow.
//!
//! The rule assumes legitimate content packs no better than four to one.
//! Packing writes a word with one non-zero byte in two bytes (exactly
//! four to one) and a run of zero words in two bytes, so a value made
//! mostly of zero words, or a decoder that reads one list twice (capnp
//! counts every read against the limit), could exceed a bound its carrier
//! admits. `at_cap_tests` holds each type's largest realistic value at its
//! carrier's cap (random keys and ciphertext, hex and record-key text,
//! small timestamps and arrival offsets, the smallest chain entries), and
//! every one decodes; a type that failed there would take a bound derived
//! from its schema, as the chain head and retire do.
//!
//! The carriers are Veilid's, from `rekindle_types::dht_layout`:
//! - `MAX_APP_MESSAGE_LEN`, 32,768 bytes: the largest `app_message` or
//!   `app_call` payload;
//! - `MAX_SUBKEY_SIZE`, 32,768 bytes: the largest value of any DHT subkey;
//! - `prekeys::MAX_SUBKEY_VALUE_LEN`, 10,381 bytes: the prekey record's
//!   per-subkey share of the record's data cap.
//!
//! The two smallest types, a chain head and a chain retire, are bounded by
//! their schema instead (`schemas/chain.capnp`), since nothing in them can
//! grow but the page key, a record key's text.

use rekindle_types::dht_layout::{member_record, prekeys, MAX_APP_MESSAGE_LEN, MAX_SUBKEY_SIZE};

/// The traversal bound for a value of at most `max_bytes` packed bytes:
/// four times that many bytes, as 8-byte words.
#[must_use]
pub const fn words_for(max_bytes: usize) -> usize {
    max_bytes / 8 * 4
}

/// The most packed bytes one word can take: a tag, its eight bytes and the
/// literal-run count that follows an all-non-zero tag.
pub const PACKED_WORD_MAX_BYTES: usize = 10;

/// A decoder's pair of bounds: [`super::unpack_bounded`] refuses packed
/// input longer than `max_bytes` before scanning it, and a message of more
/// than `max_words` words while reading it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    /// The longest packed input.
    pub max_bytes: usize,
    /// The traversal limit, in 8-byte words.
    pub max_words: usize,
}

impl Bound {
    /// The bound of a value its carrier caps at `max_bytes`:
    /// [`words_for`] that many bytes.
    #[must_use]
    pub const fn packed(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            max_words: words_for(max_bytes),
        }
    }

    /// The bound of a type whose schema caps it at `max_words` words: the
    /// segment table's word and each word at [`PACKED_WORD_MAX_BYTES`].
    #[must_use]
    pub const fn schema(max_words: usize) -> Self {
        Self {
            max_bytes: (max_words + 1) * PACKED_WORD_MAX_BYTES,
            max_words,
        }
    }
}

/// `SignedEnvelope` (`community_envelope.capnp`): the gossip wrapper, sent
/// as one `app_message` and stored as one channel subkey.
pub const SIGNED_ENVELOPE_BOUND: Bound = Bound::packed(MAX_APP_MESSAGE_LEN);

/// `CommunityEnvelope`: the signed envelope's inner payload, so never larger
/// than the envelope that carries it.
pub const COMMUNITY_ENVELOPE_BOUND: Bound = SIGNED_ENVELOPE_BOUND;

/// `GovernanceEntry` (`community_governance.capnp`): carried inside a
/// community envelope (bootstrap responses).
pub const GOVERNANCE_ENTRY_BOUND: Bound = COMMUNITY_ENVELOPE_BOUND;

/// `VoicePacket` (`voice_packet.capnp`): one media datagram, an
/// `app_message`.
pub const VOICE_PACKET_BOUND: Bound = Bound::packed(MAX_APP_MESSAGE_LEN);

/// `TransportFeedback` (`media_feedback.capnp`): one receiver report, an
/// `app_message`.
pub const TRANSPORT_FEEDBACK_BOUND: Bound = Bound::packed(MAX_APP_MESSAGE_LEN);

/// `VoiceSignaling` (`voice.capnp`): one signaling message, an
/// `app_message`.
pub const VOICE_SIGNALING_BOUND: Bound = Bound::packed(MAX_APP_MESSAGE_LEN);

/// `DmFrame` (`direct.capnp`): one 1:1 datagram, an `app_message`.
pub const DM_FRAME_BOUND: Bound = Bound::packed(MAX_APP_MESSAGE_LEN);

/// `ChainHead` (`chain.capnp`): one `PagePointer` (two `UInt64`s and a
/// record key's text, about 50 bytes) and the retire mark. The root
/// pointer, the head (one data and one pointer word), the pointer struct (two data words and one pointer word)
/// and the text are under 16 words; 64 leaves room for any key spelling.
pub const CHAIN_HEAD_BOUND: Bound = Bound::schema(64);

/// `ChainPage` (`chain.capnp`): one DHT subkey of entries.
pub const CHAIN_PAGE_BOUND: Bound = Bound::packed(MAX_SUBKEY_SIZE);

/// A successor's sealed invites page (`schemas/community_governance.capnp`
/// `SuccessorInvitesPage`), one DFLT(1) subkey.
pub const SUCCESSOR_INVITES_BOUND: Bound = Bound::packed(MAX_SUBKEY_SIZE);

/// `PresenceUpdate` and `GameStatus` (`presence.capnp`): one profile
/// subkey.
pub const PRESENCE_BOUND: Bound = Bound::packed(MAX_SUBKEY_SIZE);

/// `PrekeyRecordHead`, `OpkSlot` and `PrekeyRecordList`
/// (`identity.capnp`): one subkey of the prekey record.
pub const PREKEY_BOUND: Bound = Bound::packed(prekeys::MAX_SUBKEY_VALUE_LEN);

/// `FriendList` (`friend.capnp`): one subkey of the friend-list record.
pub const FRIEND_LIST_BOUND: Bound = Bound::packed(MAX_SUBKEY_SIZE);

/// `AccountHeader` (`account.capnp`): one subkey of the account record.
pub const ACCOUNT_HEADER_BOUND: Bound = Bound::packed(MAX_SUBKEY_SIZE);

/// `GovernanceOp` (`governance_op.capnp`): one op-log subkey of a member
/// record.
pub const GOVERNANCE_OP_BOUND: Bound = Bound::packed(crate::governance_op::GOVERNANCE_OP_MAX_BYTES);

/// `Frontier` (`governance_op.capnp`): one frontier subkey of a member
/// record.
pub const FRONTIER_BOUND: Bound = Bound::packed(member_record::MAX_SUBKEY_VALUE_LEN);

/// `GenesisCert` (`governance_op.capnp`): the anchor's one subkey.
pub const GENESIS_CERT_BOUND: Bound =
    Bound::packed(crate::governance_op::genesis::GENESIS_CERT_MAX_BYTES);

/// `PresenceBeat` and its sealed `SessionExtras` (`member_record.capnp`):
/// the beat's fixed 512 bytes, and 256 words from the schema (the padding
/// spreads its cost over the fewest words, unpacking to about 8 bytes per 7
/// packed, `member_record::beat_padding`).
pub const PRESENCE_BEAT_BOUND: Bound = Bound {
    max_bytes: crate::member_record::PRESENCE_BEAT_BYTES,
    max_words: 256,
};

/// `MemberCard` and its signed body (`member_record.capnp`): the card's
/// 8 KiB cap, and 4 Ki words.
pub const MEMBER_CARD_BOUND: Bound = Bound {
    max_bytes: crate::member_record::MEMBER_CARD_MAX_BYTES,
    max_words: 4 * 1024,
};

/// `HeadAdvanced` (`member_record.capnp`, the ring hint): its 512-byte
/// gossip cap, and 64 words.
pub const HEAD_ADVANCED_BOUND: Bound = Bound {
    max_bytes: crate::member_record::head_advanced::HEAD_ADVANCED_MAX_BYTES,
    max_words: 64,
};

/// `OutboxEntry` (`member_record.capnp`, plan E3.3): one entry alone in a
/// sealed page, so at most one subkey.
pub const OUTBOX_ENTRY_BOUND: Bound = Bound::packed(member_record::MAX_SUBKEY_VALUE_LEN);

/// `CheckpointStateRequest` (`community_governance.capnp`): its 512-byte
/// cap, and 64 words.
pub const CHECKPOINT_STATE_REQUEST_BOUND: Bound = Bound {
    max_bytes: crate::checkpoint_state::CHECKPOINT_STATE_REQUEST_MAX_BYTES,
    max_words: 64,
};

/// `CheckpointHintRequest` (`community_governance.capnp`): its cap, and
/// 64 words.
pub const CHECKPOINT_HINT_REQUEST_BOUND: Bound = Bound {
    max_bytes: crate::checkpoint_hint::CHECKPOINT_HINT_REQUEST_MAX_BYTES,
    max_words: 64,
};

/// `FrontierRequest` (`community_governance.capnp`, plan E3.2
/// S7d-keys.8): its cap, and 64 words.
pub const FRONTIER_REQUEST_BOUND: Bound = Bound {
    max_bytes: crate::frontier_request::FRONTIER_REQUEST_MAX_BYTES,
    max_words: 64,
};

/// `CheckpointHintResponse` (`community_governance.capnp`): its cap, as
/// packed bytes.
pub const CHECKPOINT_HINT_RESPONSE_BOUND: Bound =
    Bound::packed(crate::checkpoint_hint::CHECKPOINT_HINT_RESPONSE_MAX_BYTES);

/// `CheckpointStateResponse` (`community_governance.capnp`): its cap, the
/// chunk and its framing, as packed bytes.
pub const CHECKPOINT_STATE_RESPONSE_BOUND: Bound =
    Bound::packed(crate::checkpoint_state::CHECKPOINT_STATE_RESPONSE_MAX_BYTES);

/// `HistoryAd` and its sealed ranges (`member_record.capnp`): the ad's
/// 4 KiB cap, and 1 Ki words.
pub const HISTORY_AD_BOUND: Bound = Bound {
    max_bytes: crate::member_record::HISTORY_AD_MAX_BYTES,
    max_words: 1024,
};

/// `MemberRoutes` (`member_record.capnp`): its 5 KiB cap, as packed
/// bytes.
pub const MEMBER_ROUTES_BOUND: Bound = Bound::packed(crate::member_record::MEMBER_ROUTES_MAX_BYTES);

/// A packed message whose one segment claims `words` words, all zero: a
/// few hundred bytes on the wire that unpack to `words` words, the shape a
/// peer would send to exhaust a reader's memory.
#[cfg(test)]
pub(crate) fn inflating_message(words: usize) -> Vec<u8> {
    let claimed = u32::try_from(words).expect("a test size fits u32");
    let mut header = [0u8; 8];
    header[4..8].copy_from_slice(&claimed.to_le_bytes());
    // Packed: a tag naming the nonzero bytes, then those bytes.
    let mut packed = vec![0u8];
    for (i, b) in header.iter().enumerate() {
        if *b != 0 {
            packed[0] |= 1 << i;
            packed.push(*b);
        }
    }
    // Then 256 zero words, as many times as it takes, two bytes each.
    for _ in 0..=(words / 256) {
        packed.extend_from_slice(&[0x00, 0xFF]);
    }
    packed
}

#[cfg(test)]
mod at_cap_tests;

#[cfg(test)]
#[path = "bounds_tests.rs"]
mod tests;
