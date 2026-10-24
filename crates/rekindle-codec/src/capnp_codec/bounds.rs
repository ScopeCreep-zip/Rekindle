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

/// `ChainHead` (`chain.capnp`): one `PagePointer`, two `UInt64`s and a
/// record key's text (about 50 bytes). The root pointer, the head (one
/// pointer word), the pointer struct (two data words and one pointer word)
/// and the text are under 16 words; 64 leaves room for any key spelling.
pub const CHAIN_HEAD_BOUND: Bound = Bound::schema(64);

/// `ChainRetire` (`chain.capnp`): one `UInt64`, one data word.
pub const CHAIN_RETIRE_BOUND: Bound = Bound::schema(16);

/// `ChainPage` (`chain.capnp`): one DHT subkey of entries.
pub const CHAIN_PAGE_BOUND: Bound = Bound::packed(MAX_SUBKEY_SIZE);

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
/// the beat subkey's own 512-byte cap, and 256 words from the schema.
pub const PRESENCE_BEAT_BOUND: Bound = Bound {
    max_bytes: crate::member_record::PRESENCE_BEAT_MAX_BYTES,
    max_words: 256,
};

/// `MemberCard` and its signed body (`member_record.capnp`): the card's
/// 8 KiB cap, and 4 Ki words.
pub const MEMBER_CARD_BOUND: Bound = Bound {
    max_bytes: crate::member_record::MEMBER_CARD_MAX_BYTES,
    max_words: 4 * 1024,
};

/// `HistoryAd` and its sealed ranges (`member_record.capnp`): the ad's
/// 4 KiB cap, and 1 Ki words.
pub const HISTORY_AD_BOUND: Bound = Bound {
    max_bytes: crate::member_record::HISTORY_AD_MAX_BYTES,
    max_words: 1024,
};

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
mod tests {
    use super::{inflating_message, SIGNED_ENVELOPE_BOUND};
    use crate::capnp_codec::{chain, presence, voice_packet::VoicePacket};
    use crate::capnp_envelope::{
        decode_community_envelope, decode_governance_entry, decode_signed_envelope,
        encode_signed_envelope,
    };
    use crate::community::envelope::SignedEnvelope;
    use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

    fn signed(payload_len: usize) -> SignedEnvelope {
        SignedEnvelope {
            community_id: "c".into(),
            sender_pseudonym: "p".into(),
            envelope_bytes: vec![0; payload_len],
            signature: vec![0; 64],
            ttl: 5,
        }
    }

    /// The gossip envelope: a legitimate envelope whose zero payload packs
    /// into a datagram but unpacks past the bound is refused, while one the
    /// size of the whole message cap still decodes.
    #[test]
    fn the_gossip_envelope_refuses_a_message_that_inflates_past_its_bound() {
        let fits = encode_signed_envelope(&signed(MAX_APP_MESSAGE_LEN));
        assert!(decode_signed_envelope(&fits).is_ok());

        let inflating = encode_signed_envelope(&signed((SIGNED_ENVELOPE_BOUND.max_words + 1) * 8));
        assert!(inflating.len() < MAX_APP_MESSAGE_LEN, "fits one datagram");
        assert!(decode_signed_envelope(&inflating).is_err());

        let bomb = inflating_message(SIGNED_ENVELOPE_BOUND.max_words + 1);
        assert!(decode_signed_envelope(&bomb).is_err());
        assert!(decode_community_envelope(&bomb).is_err());
        assert!(decode_governance_entry(&bomb).is_err());
    }

    /// The governance op: a zero run under the subkey cap that unpacks past
    /// four times the cap is refused.
    #[test]
    fn the_governance_op_refuses_a_message_that_inflates_past_its_bound() {
        let bomb = inflating_message(super::GOVERNANCE_OP_BOUND.max_words + 1);
        assert!(bomb.len() < crate::governance_op::GOVERNANCE_OP_MAX_BYTES);
        assert!(crate::governance_op::decode_op(&bomb).is_err());
    }

    /// Input one byte over a bound's byte cap is refused before it is
    /// scanned, whatever it holds.
    #[test]
    fn input_over_the_byte_cap_is_refused_before_it_is_read() {
        let mut retire = crate::capnp_codec::chain::encode_chain_retire(7);
        assert!(chain::decode_chain_retire(&retire).is_ok());
        retire.resize(super::CHAIN_RETIRE_BOUND.max_bytes + 1, 0);
        let err = chain::decode_chain_retire(&retire).unwrap_err();
        assert!(err.to_string().contains("the bound is"), "{err}");
        let long = vec![0u8; super::SIGNED_ENVELOPE_BOUND.max_bytes + 1];
        let err = decode_signed_envelope(&long).unwrap_err();
        assert!(err.to_string().contains("the bound is"), "{err}");
    }

    /// Every other decoder refuses a message one word over its bound.
    #[test]
    fn every_decoder_refuses_a_message_one_word_over_its_bound() {
        let over = |words: usize| inflating_message(words + 1);
        assert!(chain::decode_chain_retire(&over(super::CHAIN_RETIRE_BOUND.max_words)).is_err());
        assert!(chain::decode_chain_head(&over(super::CHAIN_HEAD_BOUND.max_words)).is_err());
        assert!(chain::decode_chain_page(&over(super::CHAIN_PAGE_BOUND.max_words)).is_err());
        assert!(presence::decode_update(&over(super::PRESENCE_BOUND.max_words)).is_err());
        assert!(presence::decode_game_status(&over(super::PRESENCE_BOUND.max_words)).is_err());
        assert!(VoicePacket::decode(&over(super::VOICE_PACKET_BOUND.max_words)).is_err());
        assert!(
            crate::capnp_codec::transport_feedback::TransportFeedback::decode(&over(
                super::TRANSPORT_FEEDBACK_BOUND.max_words
            ))
            .is_err()
        );
        assert!(crate::capnp_codec::voice::decode_signaling(&over(
            super::VOICE_SIGNALING_BOUND.max_words
        ))
        .is_err());
        assert!(crate::capnp_codec::friend::decode_friend_list(&over(
            super::FRIEND_LIST_BOUND.max_words
        ))
        .is_err());
        assert!(crate::capnp_codec::account::decode_account_header(&over(
            super::ACCOUNT_HEADER_BOUND.max_words
        ))
        .is_err());
        let identity = &crate::capnp_codec::identity::decode_prekey_head;
        assert!(identity(&over(super::PREKEY_BOUND.max_words)).is_err());
        assert!(crate::capnp_codec::identity::decode_opk_slot(&over(
            super::PREKEY_BOUND.max_words
        ))
        .is_err());
        assert!(
            crate::capnp_codec::identity::decode_prekey_record_list(&over(
                super::PREKEY_BOUND.max_words
            ))
            .is_err()
        );
    }
}
