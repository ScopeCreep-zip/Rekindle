//! Inbound gossip dispatch — PATH 2 of the three-path model.
//!
//! Split out of `dispatch.rs` at its natural seam: everything here
//! decodes, verifies and delivers a *broadcast*, while the rest of that
//! module handles point-to-point traffic (app_call RPC, DMs, voice) and
//! Veilid's own update stream.
//!
//! Two formats arrive, and both are real rather than transitional:
//!
//! * framed `TypeId::GossipBroadcast` carrying a postcard
//!   `SignedGossipEnvelope` — what this track has always sent;
//! * unframed Cap'n Proto `SignedEnvelope` — what the desktop track
//!   sends, and what the daemon now sends too.
//!
//! Both authenticate the same way, by an Ed25519 signature over
//! `(community_id, sender_pseudonym, payload)`, which is why the
//! unframed one is safe to accept without a transport frame.

use std::sync::Arc;

use tracing::{trace, warn};

use crate::config::TransportConfig;
use crate::gossip::DedupCache;
use crate::handler::InboundHandler;
use crate::payload::gossip::SignedGossipEnvelope;
use crate::shared::SharedState;

pub(super) async fn dispatch_gossip<H: InboundHandler>(
    handler: &Arc<H>,
    _config: &TransportConfig,
    dedup: &mut DedupCache,
    payload: &[u8],
    _raw_frame: &[u8],
    _api: &veilid_core::VeilidAPI,
    _shared: &SharedState,
) {
    let envelope: SignedGossipEnvelope = match postcard::from_bytes(payload) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "dropping gossip: deserialization failed");
            return;
        }
    };

    if let Err(e) = crate::crypto::envelope::verify_gossip_envelope(&envelope) {
        // Phase F — promote the failure to structured fields so the
        // `RUST_LOG=rekindle_transport=debug` trace stream gives the
        // operator the community_id + sender_pseudonym + reason
        // without having to grep the unstructured message tail. The
        // transport-layer gossip envelope here does NOT carry video
        // payloads (those ride the protocol-layer `CommunityEnvelope`
        // verified in `src-tauri/services/veilid/app_message.rs`), so
        // no `VideoEvent::EnvelopeRejected` is emitted at this site —
        // only the structured warn.
        tracing::warn!(
            target: "rekindle_transport::dispatch",
            reason = %e,
            sender_pseudonym = %envelope.sender_pseudonym,
            community_id = %envelope.community_id,
            "verify_gossip_envelope failed"
        );
        return;
    }

    let dedup_key = envelope.dedup_key();
    if dedup.check_and_insert(
        &envelope.community_id,
        &envelope.sender_pseudonym,
        &dedup_key,
    ) {
        trace!(dedup_key = %dedup_key, "gossip dedup: duplicate");
        return;
    }

    if envelope.ttl > 0 && !envelope.is_private() {
        let mut forwarded = envelope.clone();
        forwarded.ttl = forwarded.ttl.saturating_sub(1);
        handler.on_gossip_forward(&forwarded).await;
    }

    let gossip_payload = match postcard::from_bytes(&envelope.payload_bytes) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "dropping gossip: inner payload parse failed");
            return;
        }
    };

    // Deliver to InboundHandler — the daemon forwards to SubscriptionManager
    handler
        .on_gossip(
            &envelope.community_id,
            &envelope.sender_pseudonym,
            gossip_payload,
            envelope.lamport_ts,
        )
        .await;
}

/// Decode, verify and deliver an unframed community gossip envelope.
///
/// Returns `true` when the bytes were a community envelope — framed or
/// malformed input falls through to the caller's existing handling.
///
/// The signature check is not optional and not deferred: an unframed
/// message has no transport-layer authentication, so `verify_envelope`
/// is the only thing standing between a forged `sender_pseudonym` and
/// the handler. Dedup runs after verification so an attacker cannot
/// poison the cache with unsigned traffic.
pub(super) async fn dispatch_bare_gossip<H: InboundHandler>(
    handler: &Arc<H>,
    dedup: &mut DedupCache,
    raw: &[u8],
) -> bool {
    use rekindle_protocol::capnp_envelope::decode_signed_envelope;
    use rekindle_protocol::capnp_envelope::try_decode_community_envelope;
    use rekindle_protocol::dht::community::envelope::verify_envelope;

    let Ok(signed) = decode_signed_envelope(raw) else {
        return false;
    };
    // A decodable `SignedEnvelope` whose inner bytes are not a
    // community envelope is not ours; say so rather than claiming the
    // message and dropping it.
    let Ok(Some(inner)) = try_decode_community_envelope(&signed.envelope_bytes) else {
        return false;
    };

    if let Err(reason) = verify_envelope(&signed) {
        warn!(
            target: "rekindle_transport::dispatch",
            %reason,
            sender_pseudonym = %signed.sender_pseudonym,
            community_id = %signed.community_id,
            "dropping unframed gossip: envelope signature rejected"
        );
        return true;
    }

    let dedup_key = rekindle_gossip::broadcast::extract_mesh_dedup_key(&inner);
    if dedup.check_and_insert(&signed.community_id, &signed.sender_pseudonym, &dedup_key) {
        trace!(dedup_key = %dedup_key, "unframed gossip dedup: duplicate");
        return true;
    }

    // `SignedEnvelope` carries `ttl` but no Lamport stamp — the
    // desktop's mesh broadcast never put one on the wire. Passing 0
    // rather than inventing a value: the receiver's clock merge takes
    // `max(ours, theirs)`, so 0 is inert, and the drift check that
    // rejects forged-future timestamps has nothing to test. Unframed
    // gossip is therefore rate-limited but not Lamport-ordered.
    //
    // Closing that gap means adding the field to the envelope, which is
    // wire-visible on both tracks; ordering within a channel still comes
    // from the `lamport_ts` inside `ChannelMessage` on PATH 1, which is
    // the copy that actually orders history.
    handler
        .on_community_gossip(&signed.community_id, &signed.sender_pseudonym, inner, 0)
        .await;
    true
}
