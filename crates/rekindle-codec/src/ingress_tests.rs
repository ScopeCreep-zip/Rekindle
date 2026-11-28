use super::*;
use crate::capnp_envelope::decode_signed_envelope;

fn signed() -> SignedEnvelope {
    SignedEnvelope {
        message_id: [7u8; 16],
        community_id: "c1".into(),
        sender_pseudonym: "ab".repeat(32),
        envelope_bytes: vec![1, 2, 3],
        signature: vec![9; 64],
        ttl: 5,
    }
}

#[test]
fn a_gossip_datagram_classifies_as_gossip_and_decodes() {
    let datagram = gossip_datagram(&signed()).unwrap();
    assert_eq!(datagram[0], GOSSIP_TAG);
    let Ingress::Gossip(bytes) = classify(&datagram) else {
        panic!("a gossip datagram must classify as gossip");
    };
    let back = decode_signed_envelope(bytes).unwrap();
    assert_eq!(back.community_id, "c1");
    assert_eq!(back.ttl, 5);
}

#[test]
fn the_tags_route_and_anything_else_is_untagged() {
    assert_eq!(classify(&[DIRECT_TAG, 7]), Ingress::Direct(&[7]));
    assert_eq!(classify(&[GOSSIP_TAG]), Ingress::Gossip(&[]));
    assert_eq!(classify(&[]), Ingress::Untagged);
    // A bare `SignedEnvelope` (the pre-E2.3 gossip wire) is untagged.
    let bare = encode_signed_envelope(&signed());
    assert_eq!(classify(&bare.unwrap()), Ingress::Untagged);
    // A transport frame starts with its version byte.
    assert_eq!(classify(&[0x01, 0, 0, 0]), Ingress::Untagged);
}
