//! The ingress tags of `app_message`/`app_call` datagrams (§6 ingress
//! ledger): the first byte names what follows, so every ingress path
//! routes by it and none trial-decodes. The media tags (`'V'`, `'M'`,
//! `'P'`, `'T'`, `'R'`) are here too (plan E4.1 S3, moved from
//! `rekindle_voice::media_frame` and `receiver_report`, which re-export
//! them), so the transport's dispatch routes media without the media
//! plane; none collides with another.
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

/// A route liveness probe (plan RH1r-c, allocated there): 16 random bytes
/// follow. Our own self-test calls our route with it and our dispatch
/// echoes a nonce it issued; any other is answered with a NAK, so a peer's
/// probe of our route learns only that the call arrived, as any call does.
pub const LIVENESS_TAG: u8 = b'L';

/// The length of a liveness probe: the tag and its nonce.
pub const LIVENESS_PROBE_LEN: usize = 17;

/// A voice packet (`VoicePacket`, Cap'n Proto) after the route's
/// `transport_seq` (`rekindle_voice::media_frame`).
pub const VOICE_TAG: u8 = b'V';
/// A signed community envelope of the video plane (video fragments,
/// parity, frame acks, keyframe requests) after the sequence number.
pub const ENVELOPE_TAG: u8 = b'M';
/// Padding the pacer sends to probe the route (plan E4.3.3): zero bytes
/// after the sequence number, counted by the estimator, discarded on
/// arrival. str0m sends blank padding the same way (`poll_packet_padding`).
pub const PADDING_TAG: u8 = b'P';
/// A `TransportFeedback` report (plan E4.3.2); no sequence number.
pub const FEEDBACK_TAG: u8 = b'T';
/// An RFC 3550 receiver report (`VoiceReceiverReport`), sibling to the
/// voice packet's tag. Reports ride the same media route as the audio they
/// describe, so they measure the path actually in use.
pub const RECEIVER_REPORT_TAG: u8 = b'R';

/// What an inbound datagram carries, by its first byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingress<'a> {
    /// [`DIRECT_TAG`]; the bytes after the tag.
    Direct(&'a [u8]),
    /// [`GOSSIP_TAG`]; the bytes after the tag.
    Gossip(&'a [u8]),
    /// A media tag ([`VOICE_TAG`], [`ENVELOPE_TAG`], [`PADDING_TAG`],
    /// [`FEEDBACK_TAG`], [`RECEIVER_REPORT_TAG`]); the whole datagram, tag
    /// included, as the media plane reads it.
    Media(&'a [u8]),
    /// Neither: not a 1:1 message or gossip.
    Untagged,
}

/// Route a datagram by its tag.
#[must_use]
pub fn classify(datagram: &[u8]) -> Ingress<'_> {
    match datagram.split_first() {
        Some((&DIRECT_TAG, rest)) => Ingress::Direct(rest),
        Some((&GOSSIP_TAG, rest)) => Ingress::Gossip(rest),
        Some((
            &(VOICE_TAG | ENVELOPE_TAG | PADDING_TAG | FEEDBACK_TAG | RECEIVER_REPORT_TAG),
            _,
        )) => Ingress::Media(datagram),
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
        // Every media tag routes as media, with the whole datagram.
        for tag in [
            VOICE_TAG,
            ENVELOPE_TAG,
            PADDING_TAG,
            FEEDBACK_TAG,
            RECEIVER_REPORT_TAG,
        ] {
            assert_eq!(classify(&[tag, 1, 2]), Ingress::Media(&[tag, 1, 2]));
        }
        assert_eq!(classify(&[LIVENESS_TAG, 0]), Ingress::Untagged);
        // A bare `SignedEnvelope` (the pre-E2.3 gossip wire) is untagged.
        let bare = encode_signed_envelope(&signed());
        assert_eq!(classify(&bare), Ingress::Untagged);
        // A transport frame starts with its version byte.
        assert_eq!(classify(&[0x01, 0, 0, 0]), Ingress::Untagged);
    }
}
