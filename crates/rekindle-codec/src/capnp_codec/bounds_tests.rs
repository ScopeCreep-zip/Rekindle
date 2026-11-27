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
        message_id: [7u8; 16],
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
    assert!(decode_signed_envelope(&fits.unwrap()).is_ok());

    let inflating = encode_signed_envelope(&signed((SIGNED_ENVELOPE_BOUND.max_words + 1) * 8)).unwrap();
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
    let mut head = chain::encode_chain_head(&chain::ChainHead {
        newest: chain::PagePointer {
            page_key: "k".into(),
            first_seq: 1,
            last_seq: 1,
        },
        retired_through: 7,
    }).unwrap();
    assert!(chain::decode_chain_head(&head).is_ok());
    head.resize(super::CHAIN_HEAD_BOUND.max_bytes + 1, 0);
    let err = chain::decode_chain_head(&head).unwrap_err();
    assert!(err.to_string().contains("the bound is"), "{err}");
    let long = vec![0u8; super::SIGNED_ENVELOPE_BOUND.max_bytes + 1];
    let err = decode_signed_envelope(&long).unwrap_err();
    assert!(err.to_string().contains("the bound is"), "{err}");
}

/// Every other decoder refuses a message one word over its bound.
#[test]
fn every_decoder_refuses_a_message_one_word_over_its_bound() {
    let over = |words: usize| inflating_message(words + 1);
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
    assert!(
        crate::capnp_codec::identity::decode_opk_slot(&over(super::PREKEY_BOUND.max_words))
            .is_err()
    );
    assert!(
        crate::capnp_codec::identity::decode_prekey_record_list(&over(
            super::PREKEY_BOUND.max_words
        ))
        .is_err()
    );
}
