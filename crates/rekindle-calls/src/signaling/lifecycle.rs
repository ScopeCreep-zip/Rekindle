//! How a 1:1 call ends (plan CE1, lead decision 22): one local end for
//! every cause, a durable `CallEnd`, and RFC 7675 consent freshness.
//!
//! * The local end comes first and never waits on the network: the call
//!   leaves the registry, `CallEnded` is emitted (the desktop destroys the
//!   call window on it), and voice is torn down. Only then is `CallEnd`
//!   committed to the outbox ([`CallSignalingDeps::commit_call_end`]),
//!   which retries it while the peer is unreachable, brings it forward when
//!   they are reachable again, and fails it at [`CALL_END_DEADLINE_MS`].
//!   The receiver is idempotent: a `CallEnd` for a call it no longer holds
//!   is a no-op.
//! * An Active call whose peer shows no media-plane proof of life (a voice
//!   packet or a verified receiver report, which arrive every ~5 s even
//!   from a VAD-silent peer; `rekindle_voice::liveness`) for
//!   [`CONSENT_TIMEOUT_MS`] ends with [`CONNECTION_LOST`] on that side
//!   alone: RFC 7675 §5.1's 30 s consent timeout, judged against the
//!   deps' clock port.

use rekindle_codec::capnp_codec::direct::{CallClosed, DirectBody};

use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::event::CallSignalEvent;
use crate::state::CallStatus;

/// RFC 7675 §5.1: consent expires 30 s after the last proof of life.
pub const CONSENT_TIMEOUT_MS: u64 = 30_000;

/// How long a `CallEnd` stays worth sending: two ring windows. A peer
/// still unreachable after that has ended the call on its own consent
/// timeout ([`CONSENT_TIMEOUT_MS`]) or ring timeout.
pub const CALL_END_DEADLINE_MS: u64 = 120_000;

/// The reason a call ended by the consent timeout carries.
pub const CONNECTION_LOST: &str = "connection-lost";

/// The reason a stale call ended by its peer's new invite carries.
pub const SUPERSEDED: &str = "superseded-by-new-call";

/// End `call_id` here: remove it, emit `CallEnded` (which retires the call
/// window), tear voice down if it was up, then commit `CallEnd` to the peer
/// through the outbox. `None` when the call is not held (already ended).
pub(crate) async fn end_locally<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    reason: &str,
) -> Option<String> {
    let removed = deps.registry().remove(call_id)?;
    let was_voice_up = matches!(removed.status, CallStatus::Active | CallStatus::Connecting);
    let peer = removed.peer_pubkey.clone();
    drop(removed); // Zeroize the call's secrets now.
    deps.emit_event(CallSignalEvent::CallEnded {
        call_id: call_id.to_string(),
        peer_public_key: peer.clone(),
        reason: reason.to_string(),
        voice_was_up: was_voice_up,
    });
    if was_voice_up {
        deps.shutdown_voice_session().await;
    }
    commit_end(deps, &peer, call_id, reason).await;
    Some(peer)
}

/// Commit `CallEnd` for `call_id` to `peer` through the outbox, due by
/// [`CALL_END_DEADLINE_MS`] from now. A commit that fails (no outbox, no
/// session to frame it) is logged with its cause: the peer then ends the
/// call on its own consent or ring timeout.
pub(crate) async fn commit_end<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    peer: &str,
    call_id: &str,
    reason: &str,
) {
    let body = DirectBody::CallEnd(CallClosed {
        call_id: call_id.to_string(),
        reason: reason.to_string(),
    });
    let deadline_ms = deps.now_ms().saturating_add(CALL_END_DEADLINE_MS);
    if let Err(e) = deps.commit_call_end(peer, body, deadline_ms).await {
        tracing::warn!(call = %call_id, peer = %peer, error = %e,
            "CallEnd not committed to the outbox; the peer ends on its own timeout");
    }
}

/// What one consent check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentCheck {
    /// The peer is live; check again in this many ms.
    Live { recheck_in_ms: u64 },
    /// Consent expired: the call was ended with [`CONNECTION_LOST`].
    Expired,
    /// The call is no longer held, or no longer Active: stop watching.
    Gone,
}

/// Check `call_id`'s consent once. `since_ms` is when it became Active
/// (the window's start before the first packet); the last proof of life is
/// the later of that and the session's latest inbound media.
pub async fn check_consent<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    call_id: &str,
    since_ms: u64,
) -> ConsentCheck {
    let active = deps
        .registry()
        .get(call_id)
        .is_some_and(|c| matches!(c.status, CallStatus::Active));
    if !active {
        return ConsentCheck::Gone;
    }
    let last = deps
        .last_inbound_media_ms()
        .map_or(since_ms, |m| m.max(since_ms));
    let silent = deps.now_ms().saturating_sub(last);
    if silent < CONSENT_TIMEOUT_MS {
        return ConsentCheck::Live {
            recheck_in_ms: CONSENT_TIMEOUT_MS - silent,
        };
    }
    tracing::info!(call = %call_id, silent_ms = silent, "call consent expired; ending");
    match end_locally(deps, call_id, CONNECTION_LOST).await {
        Some(_) => ConsentCheck::Expired,
        None => ConsentCheck::Gone,
    }
}
