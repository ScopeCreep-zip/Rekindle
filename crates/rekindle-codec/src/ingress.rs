//! The ingress tags of `app_message`/`app_call` datagrams (§6 ingress
//! ledger): the first byte names what follows, so every ingress path
//! routes by it and none trial-decodes.
//!
//! An untagged datagram is dropped. The untagged formats still accepted on
//! `app_call` (transport frames, the community capnp arms) keep their
//! order until their owners tag them (plan E2.3, "Ingress").

use crate::capnp_envelope::encode_signed_envelope;
use crate::community::envelope::SignedEnvelope;
use crate::error::CodecError;

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
    /// The full media datagram, including its tag.
    Media(&'a [u8]),
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
        Some((&(VOICE_TAG | ENVELOPE_TAG | PADDING_TAG | FEEDBACK_TAG | RECEIVER_REPORT_TAG), _)) => Ingress::Media(datagram),
        Some((&DIRECT_TAG, rest)) => Ingress::Direct(rest),
        Some((&GOSSIP_TAG, rest)) => Ingress::Gossip(rest),
        _ => Ingress::Untagged,
    }
}

/// The gossip datagram for `signed`: [`GOSSIP_TAG`] then its Cap'n Proto
/// encoding. Every gossip send and forward, on both tracks, builds its
/// bytes here.
pub fn gossip_datagram(signed: &SignedEnvelope) -> Result<Vec<u8>, CodecError> {
    let encoded = encode_signed_envelope(signed)?;
    Ok([&[GOSSIP_TAG], encoded.as_slice()].concat())
}

#[cfg(test)]
#[path = "ingress_tests.rs"]
mod tests;
