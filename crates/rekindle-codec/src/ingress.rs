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
pub fn gossip_datagram(signed: &SignedEnvelope) -> Result<Vec<u8>, CodecError> {
    let encoded = encode_signed_envelope(signed)?;
    Ok([&[GOSSIP_TAG], encoded.as_slice()].concat())
}

#[cfg(test)]
#[path = "ingress_tests.rs"]
mod tests;
