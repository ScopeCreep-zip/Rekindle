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
//! Those four arguments cover `MekTransfer`, `RequestAttachment`,
//! `BootstrapRequest` and `CheckpointStateRequest` and nothing else. Every
//! other
//! control variant carries its authority in plaintext fields — a
//! `MEKRotated` generation, a `RequestMEK` cascade index — and would be
//! forgeable by anyone who can reach our route. Those must keep
//! arriving signed, which is why this returns `None` for them and lets
//! the caller NAK.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, MekTransferPayload};

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
#[path = "bare_envelope_tests.rs"]
mod tests;
