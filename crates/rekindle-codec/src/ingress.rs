//! The ingress tags of `app_message`/`app_call` datagrams (§6 ingress
//! ledger): the first byte names what follows, so every ingress path
//! routes by it and none trial-decodes. The media tags (`'V'`, `'M'`,
//! `'P'`, `'T'`, `'R'`) live with the media plane in
//! `rekindle_voice::media_frame`; these never collide with them.
//!
//! An untagged datagram is dropped. The untagged formats still accepted on
//! `app_call` (transport frames, the community capnp arms) keep their
//! order until their owners tag them (plan E2.3, "Ingress").

use crate::capnp_envelope::encode_signed_envelope;
use crate::community::envelope::SignedEnvelope;

/// A 1:1 datagram: a `DmFrame` follows (allocated E2.1).
pub const DIRECT_TAG: u8 = b'D';

/// A community gossip datagram: a Cap'n Proto `SignedEnvelope` follows
/// (allocated E2.3; E3.2's `governanceOp`/`headAdvanced` ride it).
pub const GOSSIP_TAG: u8 = b'G';

/// What an inbound datagram carries, by its first byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingress<'a> {
    /// [`DIRECT_TAG`]; the bytes after the tag.
    Direct(&'a [u8]),
    /// [`GOSSIP_TAG`]; the bytes after the tag.
    Gossip(&'a [u8]),
    /// Neither: not a 1:1 message or gossip.
    Untagged,
}

/// Route a datagram by its tag.
#[must_use]
pub fn classify(datagram: &[u8]) -> Ingress<'_> {
    match datagram.split_first() {
        Some((&DIRECT_TAG, rest)) => Ingress::Direct(rest),
        Some((&GOSSIP_TAG, rest)) => Ingress::Gossip(rest),
        _ => Ingress::Untagged,
    }
}

/// The gossip datagram for `signed`: [`GOSSIP_TAG`] then its Cap'n Proto
/// encoding. Every gossip send and forward, on both tracks, builds its
/// bytes here.
#[must_use]
pub fn gossip_datagram(signed: &SignedEnvelope) -> Vec<u8> {
    let encoded = encode_signed_envelope(signed);
    let mut datagram = Vec::with_capacity(encoded.len() + 1);
    datagram.push(GOSSIP_TAG);
    datagram.extend_from_slice(&encoded);
    datagram
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capnp_envelope::decode_signed_envelope;

    fn signed() -> SignedEnvelope {
        SignedEnvelope {
            community_id: "c1".into(),
            sender_pseudonym: "ab".repeat(32),
            envelope_bytes: vec![1, 2, 3],
            signature: vec![9; 64],
            ttl: 5,
        }
    }

    #[test]
    fn a_gossip_datagram_classifies_as_gossip_and_decodes() {
        let datagram = gossip_datagram(&signed());
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
        assert_eq!(classify(&bare), Ingress::Untagged);
        // A transport frame starts with its version byte.
        assert_eq!(classify(&[0x01, 0, 0, 0]), Ingress::Untagged);
    }
}
