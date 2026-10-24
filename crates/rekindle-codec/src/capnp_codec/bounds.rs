//! The traversal bound of every Cap'n Proto decoder (plan F4, research
//! batch 8 AO §1).
//!
//! Every value these decoders read came off the network: a Veilid
//! `app_message`/`app_call` from a peer, or a DHT subkey any writer of the
//! record could fill. capnp's default traversal limit is 8 Mi words
//! (64 MiB), and packing collapses a run of 256 zero words into two bytes,
//! so without a bound a 32 KiB datagram could unpack to tens of megabytes.
//! [`super::unpack_bounded`] is the only reader, and each decoder passes the
//! bound below for its type.
//!
//! A bound is [`words_for`] the largest packed value its carrier delivers:
//! four times that many bytes, as words. A legitimate message unpacks to
//! about its packed size, so four times leaves room for any value that fits
//! the carrier, while a zero-run bomb is refused. This is the rule the
//! governance op, genesis cert and DM frame bounds already follow.
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

use rekindle_types::dht_layout::{prekeys, MAX_APP_MESSAGE_LEN, MAX_SUBKEY_SIZE};

/// The traversal bound for a value of at most `max_bytes` packed bytes:
/// four times that many bytes, as 8-byte words.
#[must_use]
pub const fn words_for(max_bytes: usize) -> usize {
    max_bytes / 8 * 4
}

/// `SignedEnvelope` (`community_envelope.capnp`): the gossip wrapper, sent
/// as one `app_message` and stored as one channel subkey.
pub const SIGNED_ENVELOPE_MAX_WORDS: usize = words_for(MAX_APP_MESSAGE_LEN);

/// `CommunityEnvelope`: the signed envelope's inner payload, so never larger
/// than the envelope that carries it.
pub const COMMUNITY_ENVELOPE_MAX_WORDS: usize = SIGNED_ENVELOPE_MAX_WORDS;

/// `GovernanceEntry` (`community_governance.capnp`): carried inside a
/// community envelope (bootstrap responses).
pub const GOVERNANCE_ENTRY_MAX_WORDS: usize = COMMUNITY_ENVELOPE_MAX_WORDS;

/// `VoicePacket` (`voice_packet.capnp`): one media datagram, an
/// `app_message`.
pub const VOICE_PACKET_MAX_WORDS: usize = words_for(MAX_APP_MESSAGE_LEN);

/// `TransportFeedback` (`media_feedback.capnp`): one receiver report, an
/// `app_message`.
pub const TRANSPORT_FEEDBACK_MAX_WORDS: usize = words_for(MAX_APP_MESSAGE_LEN);

/// `VoiceSignaling` (`voice.capnp`): one signaling message, an
/// `app_message`.
pub const VOICE_SIGNALING_MAX_WORDS: usize = words_for(MAX_APP_MESSAGE_LEN);

/// `ChainHead` (`chain.capnp`): one `PagePointer`, two `UInt64`s and a
/// record key's text (about 50 bytes). The root pointer, the head (one
/// pointer word), the pointer struct (two data words and one pointer word)
/// and the text are under 16 words; 64 leaves room for any key spelling.
pub const CHAIN_HEAD_MAX_WORDS: usize = 64;

/// `ChainRetire` (`chain.capnp`): one `UInt64`, one data word.
pub const CHAIN_RETIRE_MAX_WORDS: usize = 16;

/// `ChainPage` (`chain.capnp`): one DHT subkey of entries.
pub const CHAIN_PAGE_MAX_WORDS: usize = words_for(MAX_SUBKEY_SIZE);

/// `PresenceUpdate` and `GameStatus` (`presence.capnp`): one profile
/// subkey.
pub const PRESENCE_MAX_WORDS: usize = words_for(MAX_SUBKEY_SIZE);

/// `PrekeyRecordHead`, `OpkSlot` and `PrekeyRecordList`
/// (`identity.capnp`): one subkey of the prekey record.
pub const PREKEY_MAX_WORDS: usize = words_for(prekeys::MAX_SUBKEY_VALUE_LEN);

/// `FriendList` (`friend.capnp`): one subkey of the friend-list record.
pub const FRIEND_LIST_MAX_WORDS: usize = words_for(MAX_SUBKEY_SIZE);

/// `AccountHeader` (`account.capnp`): one subkey of the account record.
pub const ACCOUNT_HEADER_MAX_WORDS: usize = words_for(MAX_SUBKEY_SIZE);

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
mod tests {
    use super::{inflating_message, words_for, SIGNED_ENVELOPE_MAX_WORDS};
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

        let inflating = encode_signed_envelope(&signed((SIGNED_ENVELOPE_MAX_WORDS + 1) * 8));
        assert!(inflating.len() < MAX_APP_MESSAGE_LEN, "fits one datagram");
        assert!(decode_signed_envelope(&inflating).is_err());

        let bomb = inflating_message(SIGNED_ENVELOPE_MAX_WORDS + 1);
        assert!(decode_signed_envelope(&bomb).is_err());
        assert!(decode_community_envelope(&bomb).is_err());
        assert!(decode_governance_entry(&bomb).is_err());
    }

    /// The governance op: a zero run under the subkey cap that unpacks past
    /// four times the cap is refused.
    #[test]
    fn the_governance_op_refuses_a_message_that_inflates_past_its_bound() {
        let bomb = inflating_message(words_for(crate::governance_op::GOVERNANCE_OP_MAX_BYTES) + 1);
        assert!(bomb.len() < crate::governance_op::GOVERNANCE_OP_MAX_BYTES);
        assert!(crate::governance_op::decode_op(&bomb).is_err());
    }

    /// Every other decoder refuses a message one word over its bound.
    #[test]
    fn every_decoder_refuses_a_message_one_word_over_its_bound() {
        let over = |words: usize| inflating_message(words + 1);
        assert!(chain::decode_chain_retire(&over(super::CHAIN_RETIRE_MAX_WORDS)).is_err());
        assert!(chain::decode_chain_head(&over(super::CHAIN_HEAD_MAX_WORDS)).is_err());
        assert!(chain::decode_chain_page(&over(super::CHAIN_PAGE_MAX_WORDS)).is_err());
        assert!(presence::decode_update(&over(super::PRESENCE_MAX_WORDS)).is_err());
        assert!(presence::decode_game_status(&over(super::PRESENCE_MAX_WORDS)).is_err());
        assert!(VoicePacket::decode(&over(super::VOICE_PACKET_MAX_WORDS)).is_err());
        assert!(
            crate::capnp_codec::transport_feedback::TransportFeedback::decode(&over(
                super::TRANSPORT_FEEDBACK_MAX_WORDS
            ))
            .is_err()
        );
        assert!(crate::capnp_codec::voice::decode_signaling(&over(
            super::VOICE_SIGNALING_MAX_WORDS
        ))
        .is_err());
        assert!(crate::capnp_codec::friend::decode_friend_list(&over(
            super::FRIEND_LIST_MAX_WORDS
        ))
        .is_err());
        assert!(crate::capnp_codec::account::decode_account_header(&over(
            super::ACCOUNT_HEADER_MAX_WORDS
        ))
        .is_err());
        let identity = &crate::capnp_codec::identity::decode_prekey_head;
        assert!(identity(&over(super::PREKEY_MAX_WORDS)).is_err());
        assert!(
            crate::capnp_codec::identity::decode_opk_slot(&over(super::PREKEY_MAX_WORDS)).is_err()
        );
        assert!(
            crate::capnp_codec::identity::decode_prekey_record_list(&over(super::PREKEY_MAX_WORDS))
                .is_err()
        );
    }
}
