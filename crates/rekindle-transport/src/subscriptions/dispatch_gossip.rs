//! Inbound gossip dispatch — PATH 2 of the three-path model.
//!
//! Split out of `dispatch.rs` at its natural seam: everything here
//! decodes, verifies and delivers a *broadcast*, while the rest of that
//! module handles point-to-point traffic (app_call RPC, DMs, voice) and
//! Veilid's own update stream.
//!
//! One format: a Cap'n Proto `SignedEnvelope` after the `'G'` ingress tag
//! (§6, plan E2.3) on `app_message`. There were two — this track also sent a postcard
//! `SignedGossipEnvelope` inside a `TypeId::GossipBroadcast` frame, in
//! an enum only it could read — and the framed half is gone with the
//! enum, so a daemon and a desktop peer now exchange the same bytes.
//!
//! Accepting it unframed is safe because the envelope authenticates
//! itself: an Ed25519 signature over `(message_id, envelope_bytes)`
//! (under the gossip domain tag) made with the sender's community
//! pseudonym, whose public key is `sender_pseudonym`. The
//! frame's signature would be redundant with a property the payload
//! already carries.

use std::sync::Arc;

use rekindle_codec::community::envelope::SignedEnvelope;
use tracing::{trace, warn};

use crate::gossip::DedupCache;
use crate::handler::InboundHandler;

/// Verify and deliver a community gossip envelope on the gossip worker.
///
/// The signature check is not optional and not deferred: a gossip
/// datagram has no transport-layer authentication, so `verify_envelope`
/// is the only thing standing between a forged `sender_pseudonym` and
/// the handler. Dedup runs after verification so an attacker cannot
/// poison the cache with unsigned traffic.
pub(super) async fn dispatch_bare_gossip<H: InboundHandler>(
    handler: &Arc<H>,
    dedup: &mut DedupCache,
    signed: SignedEnvelope,
) {
    use rekindle_codec::capnp_envelope::try_decode_community_envelope;
    use rekindle_codec::community::envelope::verify_envelope;

    let inner = match try_decode_community_envelope(&signed.envelope_bytes) {
        Ok(Some(inner)) => inner,
        Ok(None) => {
            trace!(
                community_id = %signed.community_id,
                "dropping gossip: unknown envelope variant"
            );
            return;
        }
        Err(e) => {
            warn!(error = %e, "dropping gossip: malformed community envelope");
            return;
        }
    };

    if let Err(reason) = verify_envelope(&signed) {
        warn!(
            target: "rekindle_transport::dispatch",
            %reason,
            sender_pseudonym = %signed.sender_pseudonym,
            community_id = %signed.community_id,
            "dropping gossip: envelope signature rejected"
        );
        return;
    }

    // Keyed by the signed message id (one message, every relayed copy),
    // never by content: a deliberate re-send of identical bytes carries a
    // new id and is delivered; receivers are idempotent.
    let dedup_key = rekindle_codec::dedup::message_dedup_key(&signed);
    if dedup.check_and_insert(&signed.community_id, &signed.sender_pseudonym, &dedup_key) {
        trace!(dedup_key = %dedup_key, "gossip dedup: duplicate");
        return;
    }

    // Re-broadcast before processing. Epidemic delivery depends on it:
    // a sender fans out to `min(N, 6)` peers, and the architecture's
    // guarantee is "the dedup cache + 5-hop TTL deliver without flooding
    // even when most peers don't receive a direct copy". Without this
    // hop the wave stops at the sender's fan-out and a 40-member
    // community sees a message reach six people.
    //
    // Forwarded first, as the framed path does, so a slow handler does
    // not delay the wave. `is_directed` payloads are never amplified —
    // §10.6, and see that method for the two definitions it replaced.
    // A head hint is gated by the member's handler before it is relayed.
    if let Some(rekindle_codec::community::envelope::ControlPayload::HeadAdvanced { hint }) =
        inner.as_control()
    {
        let relay = handler
            .on_head_advanced(&signed.community_id, &signed.sender_pseudonym, hint)
            .await;
        if relay && signed.ttl > 0 {
            let mut forwarded = signed.clone();
            forwarded.ttl = forwarded.ttl.saturating_sub(1);
            handler.on_gossip_forward(&forwarded).await;
        }
        return;
    }

    if signed.ttl > 0 && !inner.is_directed() {
        let mut forwarded = signed.clone();
        forwarded.ttl = forwarded.ttl.saturating_sub(1);
        handler.on_gossip_forward(&forwarded).await;
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
        .on_gossip(&signed.community_id, &signed.sender_pseudonym, inner, 0)
        .await;
}
