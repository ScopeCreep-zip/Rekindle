//! Every public decoder of peer bytes, fed garbage and random Cap'n Proto
//! messages, returns (gate hardening 3(a): a panic on peer input is a
//! remote crash). F4's fuzz smoke pulled forward as a property test while
//! the tree has no `fuzz/` targets.

use proptest::prelude::*;
use rekindle_codec::capnp_codec::direct::{DmContent, DmFrame, SealedContent};
use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use rekindle_codec::capnp_codec::voice_packet::VoicePacket;
use rekindle_codec::capnp_codec::{account, chain, friend, identity, presence, voice};
use rekindle_codec::{
    capnp_envelope, checkpoint_hint, checkpoint_state, frontier_request, governance_op,
    successor_invites,
};

/// Run every single-input decoder over `bytes`; each may refuse, none may
/// panic.
fn decode_all(bytes: &[u8]) {
    let _ = presence::decode_update(bytes);
    let _ = presence::decode_game_status(bytes);
    let _ = voice::decode_signaling(bytes);
    let _ = DmFrame::decode(bytes);
    let _ = DmContent::decode(bytes);
    let _ = SealedContent::decode(bytes);
    let _ = TransportFeedback::decode(bytes);
    let _ = VoicePacket::decode(bytes);
    let _ = identity::decode_prekey_head(bytes);
    let _ = identity::decode_opk_slot(bytes);
    let _ = identity::decode_prekey_record_list(bytes);
    let _ = account::decode_account_header(bytes);
    let _ = chain::decode_chain_head(bytes);
    let _ = chain::decode_chain_page(bytes);
    let _ = friend::decode_friend_list(bytes);
    let _ = frontier_request::decode_frontier_request(bytes);
    let _ = frontier_request::decode_frontier_response(bytes);
    let _ = checkpoint_state::decode_checkpoint_state_request(bytes);
    let _ = checkpoint_state::decode_checkpoint_state_response(bytes);
    let _ = checkpoint_hint::decode_checkpoint_hint_request(bytes);
    let _ = checkpoint_hint::decode_checkpoint_hint_response(bytes);
    let _ = successor_invites::decode_invites_page(bytes);
    let _ = governance_op::decode_op(bytes);
    let _ = governance_op::decode_frontier(bytes);
    let _ = capnp_envelope::decode_community_envelope(bytes);
    let _ = capnp_envelope::try_decode_community_envelope(bytes);
    let _ = capnp_envelope::decode_signed_envelope(bytes);
    let _ = capnp_envelope::decode_governance_entry(bytes);
    let _ = rekindle_codec::member_record::head_advanced::decode_head_advanced(
        bytes,
        "community",
        &[7; 32],
    );
}

/// A well-formed packed single-segment message of random words: every
/// word written literally (tag `0xff`, its bytes, a zero run count), so
/// decoders get past unpacking and follow random pointers.
fn random_message(words: &[u64]) -> Vec<u8> {
    let count = u32::try_from(words.len()).unwrap();
    let table = [0u32.to_le_bytes(), count.to_le_bytes()].concat();
    let mut out = Vec::new();
    for word in std::iter::once(table.as_slice()).chain(
        words
            .iter()
            .map(|w| w.to_le_bytes())
            .collect::<Vec<_>>()
            .iter()
            .map(<[u8; 8]>::as_slice),
    ) {
        out.push(0xff);
        out.extend_from_slice(word);
        out.push(0);
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn garbage_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        decode_all(&bytes);
    }

    #[test]
    fn random_messages_never_panic(words in proptest::collection::vec(any::<u64>(), 1..96)) {
        decode_all(&random_message(&words));
    }

    #[test]
    fn invite_urls_never_panic(url in "\\PC{0,512}") {
        let _ = rekindle_codec::message::decode_invite_url(&url);
        let _ = rekindle_codec::message::decode_invite_url(
            &format!("rekindle://invite/{url}"),
        );
    }
}
