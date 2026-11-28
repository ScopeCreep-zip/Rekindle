use super::{
    decode_bare_attachment_request, decode_bare_bootstrap_request,
    decode_bare_checkpoint_hint_request, decode_bare_checkpoint_state_request,
    decode_bare_frontier_request, decode_bare_media_key, decode_bare_mek_transfer,
};
use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, MekTransferPayload};

fn mek_transfer_bytes() -> Vec<u8> {
    encode_community_envelope(&CommunityEnvelope::control(ControlPayload::MekTransfer(
        MekTransferPayload {
            community_id: "gov-key".into(),
            channel_id: Some("general".into()),
            generation: 7,
            sender_pseudonym: "aa".repeat(32),
            wrapped_mek: vec![1, 2, 3, 4],
            transfer_id: vec![9; 16],
        },
    )))
    .expect("encode")
}

#[test]
fn admits_a_wrapped_mek() {
    let decoded = decode_bare_mek_transfer(&mek_transfer_bytes()).expect("should admit");
    assert_eq!(decoded.community_id, "gov-key");
    assert_eq!(decoded.generation, 7);
    assert_eq!(decoded.wrapped_mek, vec![1, 2, 3, 4]);
}

/// The security boundary. `MEKRotated` states a generation in
/// plaintext, so accepting it unsigned would let anyone who can
/// reach our route claim a rotation happened. It must fall through
/// to the NAK path even though it is a perfectly valid envelope.
#[test]
fn refuses_other_control_variants_even_when_well_formed() {
    let rotated =
        encode_community_envelope(&CommunityEnvelope::control(ControlPayload::MEKRotated {
            channel_id: Some("general".into()),
            new_generation: 9,
            rotator_pseudonym: Some("bb".repeat(32)),
        }))
        .expect("encode");
    assert!(
        decode_bare_mek_transfer(&rotated).is_none(),
        "an unsigned MEKRotated must not be actionable"
    );
    assert!(decode_bare_media_key(&rotated).is_none());
    assert!(decode_bare_attachment_request(&rotated).is_none());
    assert!(decode_bare_bootstrap_request(&rotated).is_none());
    assert!(decode_bare_checkpoint_state_request(&rotated).is_none());
    assert!(decode_bare_checkpoint_hint_request(&rotated).is_none());
    assert!(decode_bare_frontier_request(&rotated).is_none());
}

#[test]
fn admits_an_attachment_request_and_nothing_else_as_one() {
    let request = encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::RequestAttachment {
            channel_id: "general".into(),
            attachment_id: [7; 16],
            requested_chunks: vec![0, 2],
            requester_pseudonym: "cc".repeat(32),
        },
    ))
    .expect("encode");
    assert_eq!(
        decode_bare_attachment_request(&request),
        Some(([7; 16], vec![0, 2]))
    );
    assert!(decode_bare_mek_transfer(&request).is_none());
    assert!(decode_bare_attachment_request(&mek_transfer_bytes()).is_none());
    assert!(decode_bare_attachment_request(b"garbage").is_none());
}

#[test]
fn refuses_garbage() {
    assert!(decode_bare_mek_transfer(b"not an envelope at all").is_none());
    assert!(decode_bare_mek_transfer(&[]).is_none());
}

/// Truncation must not be mistaken for a valid short envelope.
#[test]
fn refuses_truncated_envelope() {
    let full = mek_transfer_bytes();
    for cut in [1, full.len() / 3, full.len() / 2, full.len() - 1] {
        assert!(
            decode_bare_mek_transfer(&full[..cut]).is_none(),
            "truncation at {cut} must not decode"
        );
    }
}
