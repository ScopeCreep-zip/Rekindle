//! The one unframed inbound format this track accepts.
//!
//! Every other `app_call` must arrive as a transport frame carrying a
//! `SignedPayload`, and `dispatch_app_call` verifies that signature
//! before the handler sees anything. A wrapped MEK is the single
//! exception: the desktop track sends it as a bare Cap'n Proto
//! `CommunityEnvelope` (`services/veilid/network.rs` reads
//! `call.message()` and decodes it directly), so without this the two
//! shells NAK each other's format and a rotated key never crosses
//! between them.
//!
//! Admitting it is safe *because of what the payload is*, not because
//! the sender asserts anything: `rekindle_secrets::mek::unwrap_mek`
//! takes the sender's pseudonym public key as an ECDH input, so
//! ciphertext only decrypts if the named sender actually produced it.
//! The `sender_pseudonym` field is bound by the cryptography rather
//! than believed.
//!
//! The second admitted format is a `RequestAttachment` (architecture
//! §28.9), which `rekindle_files::download` sends bare to whichever peer
//! advertised the chunks. It carries no authority at all: it asks for
//! bytes and changes nothing, and the bytes it gets back are chunk
//! ciphertext under the attachment's FEK, which only a member holding
//! the channel key can unwrap.
//!
//! The third is a joiner's `BootstrapRequest` (plan E3.2). Its fields
//! claim nothing the handler believes: it names the joiner's op #0 by
//! `OpRef`, and the handler fetches that op from the joiner's member
//! record, checks its hash and signature, and merges it under the
//! community's rules before it answers; the answer (a frontier of op
//! locators) is what any member's `Frontier` subkey already publishes.
//!
//! The fourth is a member's `CheckpointStateRequest` (plan E3.2 S7c.3).
//! It is signed by the asking member's pseudonym inside the payload, which
//! the handler verifies and admits as a member before it answers; the
//! answer is bytes the reader checks against the endorsed `stateRoot`.
//!
//! The fifth is a call-media key (plan E4.1 S3): a `VoiceMediaKey` or a
//! `VoiceMediaKeyRequest`, the bare envelopes the desktop's voice sends
//! point to point (plan C7.22, ADR 0016). Each carries its own
//! authority: a key is sealed to the recipient and signed by its sender's
//! pseudonym inside the payload (plan C7.29), and a request names the key
//! index it asks for, which the handler answers only with a key sealed to
//! the requester, so neither is forgeable by reaching our route. What
//! reaching our route does buy is volume: each one makes the handler
//! verify a signature (and, for a request, seal a key), so they are
//! metered per requester ([`MediaKeyAdmission`]) before the handler
//! sees them, the receiver-side floor gossip has (architecture §20.2).
//!
//! Those five arguments cover `MekTransfer`, `RequestAttachment`,
//! `BootstrapRequest`, `CheckpointStateRequest` (the hint and frontier
//! requests are routed like it) and the two media-key variants, and
//! nothing else. Every other control variant carries its authority in plaintext fields — a
//! `MEKRotated` generation, a `RequestMEK` cascade index — and would be
//! forgeable by anyone who can reach our route. Those must keep
//! arriving signed, which is why this returns `None` for them and lets
//! the caller NAK.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, MekTransferPayload};
use rekindle_gossip::rate_limit::TokenBucket;

/// Recognise a bare `VoiceMediaKey` or `VoiceMediaKeyRequest` (plan E4.1
/// S3), and only those: the payload and the requester it is metered by
/// (a key's `sender`, a request's `requester`).
pub(super) fn decode_bare_media_key(raw: &[u8]) -> Option<(String, ControlPayload)> {
    let payload = rekindle_codec::capnp_envelope::try_decode_community_envelope(raw)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)?;
    let requester = match &payload {
        ControlPayload::VoiceMediaKey { sender, .. } => sender.clone(),
        ControlPayload::VoiceMediaKeyRequest { requester, .. } => requester.clone(),
        _ => return None,
    };
    Some((requester, payload))
}

/// Burst of media-key calls one requester may make: a sender delivers one
/// key per recipient per rotation (ratchet-on-join, rotation on every
/// departure, ADR 0016 / plan C7.29), and a joiner asks once per sender it
/// lacks a key for, so a burst is a handful.
pub const MEDIA_KEY_BURST: u32 = 8;
/// Media-key calls per second one requester sustains past its burst.
pub const MEDIA_KEY_PER_SEC: u32 = 2;
/// Buckets held before idle ones are pruned: the requester is a claim, so
/// a flood of invented names must not pin the map.
const MEDIA_KEY_SOFT_CAP: usize = 1024;
/// A bucket idle this long is pruned.
const MEDIA_KEY_IDLE_TTL: Duration = Duration::from_secs(60);

/// The per-requester token bucket the bare media-key branch meters by
/// (plan E4.1 S3): over its floor a requester's call is refused before
/// any handler work.
#[derive(Debug, Default)]
pub struct MediaKeyAdmission {
    buckets: HashMap<String, TokenBucket>,
}

impl MediaKeyAdmission {
    /// Take one token for `requester` at `now`; `false` refuses the call.
    pub fn admit(&mut self, requester: &str, now: Instant) -> bool {
        let admitted = self
            .buckets
            .entry(requester.to_string())
            .or_insert_with(|| TokenBucket::new(MEDIA_KEY_BURST, MEDIA_KEY_PER_SEC, now))
            .try_consume_at(1, now);
        if self.buckets.len() > MEDIA_KEY_SOFT_CAP {
            self.buckets
                .retain(|_, b| now.saturating_duration_since(b.last_refill()) < MEDIA_KEY_IDLE_TTL);
        }
        admitted
    }
}

/// Recognise a bare `CommunityEnvelope` carrying a wrapped MEK.
///
/// A *typed* discriminator, not byte-sniffing:
/// `try_decode_community_envelope` returns `Err` on malformed bytes and
/// `Ok(None)` on a union discriminant it does not know, so anything
/// that is not genuinely this envelope falls through to `None`.
pub(super) fn decode_bare_mek_transfer(raw: &[u8]) -> Option<MekTransferPayload> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(raw)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::MekTransfer(transfer)) => Some(transfer),
        _ => None,
    }
}

/// Recognise a bare `CommunityEnvelope` asking for attachment chunks:
/// the attachment id and the chunk indices requested.
pub(super) fn decode_bare_attachment_request(raw: &[u8]) -> Option<([u8; 16], Vec<u32>)> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(raw)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::RequestAttachment {
            attachment_id,
            requested_chunks,
            ..
        }) => Some((attachment_id, requested_chunks)),
        _ => None,
    }
}

/// Recognise a bare `CommunityEnvelope` carrying a joiner's
/// `BootstrapRequest`.
pub(super) fn decode_bare_bootstrap_request(
    raw: &[u8],
) -> Option<crate::payload::rpc::InboundCall> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(raw)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::BootstrapRequest {
            joiner_pseudonym,
            anchor,
            member_joined,
            member_joined_op,
            joiner_route,
        }) => Some(crate::payload::rpc::InboundCall::BootstrapRequest {
            joiner_pseudonym,
            anchor,
            member_joined,
            member_joined_op,
            joiner_route,
        }),
        _ => None,
    }
}

/// Recognise a bare `CommunityEnvelope` carrying a member's
/// `CheckpointStateRequest`.
pub(super) fn decode_bare_checkpoint_state_request(
    raw: &[u8],
) -> Option<crate::payload::rpc::InboundCall> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(raw)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::CheckpointStateRequest { request }) => {
            Some(crate::payload::rpc::InboundCall::CheckpointStateRequest { request })
        }
        _ => None,
    }
}

/// A bare `checkpointHintRequest` (plan E3.2 S7c.5), routed like the
/// snapshot request.
pub(super) fn decode_bare_checkpoint_hint_request(
    raw: &[u8],
) -> Option<crate::payload::rpc::InboundCall> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(raw)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::CheckpointHintRequest { request }) => {
            Some(crate::payload::rpc::InboundCall::CheckpointHintRequest { request })
        }
        _ => None,
    }
}

/// A bare `frontierRequest` (plan E3.2 S7d-keys.8), routed like the
/// snapshot request.
pub(super) fn decode_bare_frontier_request(raw: &[u8]) -> Option<crate::payload::rpc::InboundCall> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(raw)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::FrontierRequest { request }) => {
            Some(crate::payload::rpc::InboundCall::FrontierRequest { request })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_bare_attachment_request, decode_bare_bootstrap_request,
        decode_bare_checkpoint_hint_request, decode_bare_checkpoint_state_request,
        decode_bare_frontier_request, decode_bare_media_key, decode_bare_mek_transfer,
    };
    use rekindle_codec::capnp_envelope::encode_community_envelope;
    use rekindle_codec::community::envelope::{
        CommunityEnvelope, ControlPayload, MekTransferPayload,
    };

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
        // No bare branch admits it, the media-key branch included.
        assert!(decode_bare_media_key(&rotated).is_none());
        assert!(decode_bare_attachment_request(&rotated).is_none());
        assert!(decode_bare_bootstrap_request(&rotated).is_none());
        assert!(decode_bare_checkpoint_state_request(&rotated).is_none());
        assert!(decode_bare_checkpoint_hint_request(&rotated).is_none());
        assert!(decode_bare_frontier_request(&rotated).is_none());
    }

    fn control_bytes(payload: ControlPayload) -> Vec<u8> {
        encode_community_envelope(&CommunityEnvelope::control(payload)).expect("encode")
    }

    /// The media-key branch admits a key and a request, metered by the
    /// key's sender and the request's requester, and nothing else: the
    /// key's ack (also a valid media-key envelope), the MEK transfer and
    /// an attachment request all fall through to their own branch or the
    /// NAK.
    #[test]
    fn the_media_key_branch_admits_only_a_key_and_a_request() {
        let key = control_bytes(ControlPayload::VoiceMediaKey {
            community_id: "c".into(),
            channel_id: "ch".into(),
            sender: "s".into(),
            recipient: "r".into(),
            key_index: 4,
            sealed: vec![7; 80],
            signature: vec![8; 64],
        });
        let request = control_bytes(ControlPayload::VoiceMediaKeyRequest {
            community_id: "c".into(),
            channel_id: "ch".into(),
            requester: "q".into(),
            sender: "s".into(),
            key_index: 3,
        });
        let ack = control_bytes(ControlPayload::VoiceMediaKeyAck {
            community_id: "c".into(),
            channel_id: "ch".into(),
            sender: "s".into(),
            recipient: "r".into(),
            key_index: 9,
        });
        let attachment = control_bytes(ControlPayload::RequestAttachment {
            channel_id: "general".into(),
            attachment_id: [7; 16],
            requested_chunks: vec![0],
            requester_pseudonym: "cc".repeat(32),
        });
        assert_eq!(
            decode_bare_media_key(&key).map(|(who, _)| who),
            Some("s".into())
        );
        assert_eq!(
            decode_bare_media_key(&request).map(|(who, _)| who),
            Some("q".into())
        );
        for refused in [ack, mek_transfer_bytes(), attachment, b"garbage".to_vec()] {
            assert!(decode_bare_media_key(&refused).is_none());
        }
        assert!(decode_bare_mek_transfer(&key).is_none());
        assert!(decode_bare_attachment_request(&request).is_none());
    }

    /// Each requester has its own bucket: one past its burst is refused
    /// while another requester is still admitted, and a refused one is
    /// admitted again once its bucket refills.
    #[test]
    fn media_keys_are_rate_limited_per_requester() {
        use super::{MediaKeyAdmission, MEDIA_KEY_BURST, MEDIA_KEY_PER_SEC};
        let now = std::time::Instant::now();
        let mut admission = MediaKeyAdmission::default();
        for _ in 0..MEDIA_KEY_BURST {
            assert!(admission.admit("flooder", now));
        }
        assert!(!admission.admit("flooder", now), "past its burst");
        assert!(
            admission.admit("honest", now),
            "another requester's own bucket"
        );
        let refilled = now + std::time::Duration::from_millis(1_000 / u64::from(MEDIA_KEY_PER_SEC));
        assert!(admission.admit("flooder", refilled));
        assert!(!admission.admit("flooder", refilled));
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
}
