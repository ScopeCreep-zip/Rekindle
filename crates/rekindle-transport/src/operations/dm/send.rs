//! Sending one 1:1 frame: the cached route, an inline route refresh from
//! the peer's profile, a status probe (§13.5), and finally the
//! pending-message queue. Every send goes through the node's ports; the
//! bytes are [`frame::wire`]. The Strand Relay fallback (§13.3) returns at
//! E5.5, whose pool entries name the relay's identity: a relay frame is
//! addressed and signed to it, and a bare route blob names no one.
//!
//! What the queue holds until E2.5 (D22): a ratchet or PreKeyInit frame's
//! wire bytes exactly as encrypted and recorded (retried byte-identical:
//! the ratchet state was committed before the first send, and the receiver
//! drops a used `(dh, n)`), and a sealed frame's body and nonce, re-sealed
//! with a fresh `sent_at` per attempt (the replay guard drops a copy that
//! already arrived).

use rekindle_codec::capnp_codec::direct::{Carrier, ContentHint, DirectBody, DmFrame, DmTypeId};
use serde::{Deserialize, Serialize};

use super::frame::{self, Nonce};
use super::route;
use crate::operations::direct::frame::Opened;
use crate::operations::friend::{state, FriendError, FriendHost};

/// A queued frame (`pending_messages.body`, JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Queued {
    /// A ratchet or PreKeyInit frame's datagram, sent again as it is.
    Wire {
        #[serde(with = "rekindle_types::base64_bytes")]
        datagram: Vec<u8>,
    },
    /// A sealed kind's body, re-sealed per attempt under its nonce.
    Sealed {
        type_id: u16,
        #[serde(with = "rekindle_types::base64_bytes")]
        body: Vec<u8>,
        nonce: Nonce,
    },
}

/// The D24 content hint of a kind: a text message may be resent from the
/// send log (E2.5), typing is implicit (its loss shows nothing), and the
/// rest are the default.
#[must_use]
pub fn hint_for(type_id: DmTypeId) -> ContentHint {
    match type_id {
        DmTypeId::DmText => ContentHint::Resendable,
        DmTypeId::Typing => ContentHint::Implicit,
        _ => ContentHint::Default,
    }
}

/// `datagram` over the peer's freshly fetched route.
async fn refresh_and_send(host: &dyn FriendHost, to: &str, datagram: &[u8]) -> bool {
    let Some(blob) = route::fetch_from_profile(host, to).await else {
        return false;
    };
    let Ok(node) = host.node() else {
        return false;
    };
    let Some(target) = route::import(&node, to, &blob) else {
        return false;
    };
    node.sender()
        .send_direct(&target, datagram.to_vec())
        .await
        .is_ok()
}

/// Ask friends in the background whether any holds a fresh status and
/// route for `to` (architecture §13.5); a late answer refreshes the cache
/// for the next send.
fn probe_in_background(host: &dyn FriendHost, to: &str) {
    let owned = host.owned();
    let target = to.to_string();
    host.scope()
        .spawn_or_drop("relay status probe", async move {
            crate::operations::relay::probe_friends_for_status(owned.as_ref(), &target).await;
        });
}

/// Queue `queued` for the pending-message retry.
///
/// # Errors
/// The queue could not be written.
async fn queue(host: &dyn FriendHost, to: &str, queued: &Queued) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let body = queued_json(queued)?;
    let recipient = to.to_string();
    let now = host.clock().now_ms_i64();
    state::db_call!(host, "queue pending message", move |conn| {
        rekindle_db::repo::pending_messages::insert(conn, &owner, &recipient, &body, now)
    })
    .await
}

fn queued_json(queued: &Queued) -> Result<String, FriendError> {
    serde_json::to_string(queued)
        .map_err(|e| FriendError::Failed(format!("serialize queued frame: {e}")))
}

/// `body` to `to` as the pending-message queue stores it, for a caller
/// that queues it in a transaction of its own (a removal, plan audit D-1).
///
/// # Errors
/// The frame could not be built.
pub async fn queued_form(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<String, FriendError> {
    let (_, queued) = framed(host, to, body).await?;
    queued_json(&queued)
}

/// The wired `app_call` reply carrying `body` to `to`, framed as any send
/// of its kind (the verification gate, the kind's carrier and hint).
///
/// # Errors
/// The frame could not be built.
pub async fn reply_frame(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<Vec<u8>, FriendError> {
    let (frame, _) = framed(host, to, body).await?;
    frame::wire(&frame)
}

/// The frame to `to` for `body`, and how the queue keeps it.
async fn framed(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<(DmFrame, Queued), FriendError> {
    let type_id = body.type_id();
    if !crate::operations::friend::verification::is_protective(type_id) {
        crate::operations::friend::verification::require_acknowledged(host, to).await?;
    }
    if type_id.carrier() == Carrier::Sealed {
        let mut nonce = Nonce::default();
        host.rng().fill(&mut nonce);
        let frame = frame::sealed_frame(host, to, body, nonce)?;
        let queued = Queued::Sealed {
            type_id: type_id.as_u16(),
            body: body.encode(),
            nonce,
        };
        return Ok((frame, queued));
    }
    let frame = frame::frame(host, to, body, hint_for(type_id)).await?;
    let queued = Queued::Wire {
        datagram: frame::wire(&frame)?,
    };
    Ok((frame, queued))
}

/// Send `body` to `to` as its kind travels.
///
/// With no usable route the frame is refreshed inline, and otherwise
/// queued for the retry. An ephemeral kind (typing, a call signal, DM
/// video: `DmTypeId::is_ephemeral`) is never queued: minutes late it is
/// worse than none.
///
/// # Errors
/// The frame could not be sealed (no session for a ratchet kind), or it is
/// over Veilid's message cap (never queued: it fails on every path, plan
/// C7.25).
pub async fn send_to_peer(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<(), FriendError> {
    let (frame, queued) = framed(host, to, body).await?;
    deliver(host, to, &frame, &queued).await
}

/// Send a frame built elsewhere (a PreKeyInit opened from a bundle) to
/// `to`, queued like any other when no route takes it.
///
/// # Errors
/// The frame is over Veilid's message cap, or the queue could not be
/// written.
pub async fn send_frame(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
) -> Result<(), FriendError> {
    let queued = Queued::Wire {
        datagram: frame::wire(frame)?,
    };
    deliver(host, to, frame, &queued).await
}

/// One send of `frame`: the cached route, then (with `refresh`; never for
/// an ephemeral kind) a route refreshed from the peer's profile, with a
/// status probe when none was cached. Whether it left this device.
///
/// # Errors
/// The frame is over Veilid's message cap, or the node is not up.
pub(crate) async fn route_send(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
    refresh: bool,
) -> Result<bool, FriendError> {
    let datagram = frame::wire(frame)?;
    let node = host.node()?;
    let failure = match route::cached_target(&node, to) {
        Some(target) => match node.sender().send_direct(&target, datagram.clone()).await {
            Ok(()) => {
                tracing::debug!(to = %to, kind = ?frame.type_id, "frame sent via veilid");
                return Ok(true);
            }
            Err(e) => {
                // Only an unusable route goes (plan audit G10), so the
                // next attempt fetches a fresh one; a send that failed for
                // any other reason keeps it.
                if matches!(e, crate::error::TransportError::RouteUnusable { .. }) {
                    route::invalidate(&node, to);
                }
                Some(e.to_string())
            }
        },
        None => None,
    };
    if !refresh {
        tracing::debug!(to = %to, error = ?failure, "no route for peer — dropping ephemeral frame");
        return Ok(false);
    }
    if refresh_and_send(host, to, &datagram).await {
        tracing::debug!(to = %to, "frame sent via veilid (after inline route refresh)");
        return Ok(true);
    }
    if failure.is_none() {
        probe_in_background(host, to);
    }
    tracing::debug!(to = %to, error = ?failure, "no usable route for peer");
    Ok(false)
}

/// `body` framed as entry `stream_seq` of our outbox stream to `to` (plan
/// E2.5): a ratchet message carrying the seq the peer's `DmAck` names,
/// behind the verification gate like any send of its kind.
///
/// # Errors
/// The kind does not travel on the ratchet, the gate holds it, or there is
/// no session.
pub(crate) async fn stream_frame(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
    stream_seq: u64,
) -> Result<DmFrame, FriendError> {
    let type_id = body.type_id();
    if type_id.carrier() != Carrier::Ratchet {
        return Err(FriendError::Invalid(format!(
            "{type_id:?} does not travel on the DM stream"
        )));
    }
    if !crate::operations::friend::verification::is_protective(type_id) {
        crate::operations::friend::verification::require_acknowledged(host, to).await?;
    }
    frame::ratchet_frame_at(host, to, body, hint_for(type_id), stream_seq).await
}

/// Send `body` to `to` once and never queue it: a receipt the peer's resend
/// asks for again (`DmAck`, plan E2.5). Whether it left this device.
///
/// # Errors
/// The frame could not be built or is over the cap.
pub(crate) async fn send_once(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<bool, FriendError> {
    let (frame, _) = framed(host, to, body).await?;
    route_send(host, to, &frame, true).await
}

/// One send of `frame`, queued last unless its kind is ephemeral.
async fn deliver(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
    queued: &Queued,
) -> Result<(), FriendError> {
    let ephemeral = frame.type_id.is_ephemeral();
    if route_send(host, to, frame, !ephemeral).await? || ephemeral {
        return Ok(());
    }
    tracing::debug!(to = %to, "queuing frame for retry");
    queue(host, to, queued).await
}

/// Frame `body` and queue it for the retry without trying to send it now:
/// a notice whose delivery the queue must track (`Unfriended` until its
/// ack).
///
/// # Errors
/// The frame could not be sealed, or the queue could not be written.
pub async fn build_and_queue(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<(), FriendError> {
    let (_, queued) = framed(host, to, body).await?;
    queue(host, to, &queued).await
}

/// `app_call` `body` to `to` and open the reply: a frame from `to`, or
/// `None` for an empty reply (the call's return is the receipt).
///
/// # Errors
/// No cached route, the call failed, the peer refused it (a NAK), or the
/// reply is not a frame from `to` that opens.
pub async fn send_to_peer_call(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<Option<Opened>, FriendError> {
    let (frame, _) = framed(host, to, body).await?;
    call_frame(host, to, &frame).await
}

/// What [`call_or_send`] came to.
#[derive(Debug)]
pub enum Delivery {
    /// The call returned its receipt.
    Receipted,
    /// The call did not land; the same frame went down the send fallbacks
    /// (sent, or queued).
    Sent,
}

/// `app_call` `body` to `to`; when no receipt comes back (no route, a
/// timeout, a NAK), send the same frame like any other. A sealed kind
/// keeps the call's nonce on the fallback and in the queue, so a copy of a
/// call that did land is dropped by the receiver's replay guard (plan
/// audit G4).
///
/// # Errors
/// The frame could not be sealed, or the fallback failed.
pub async fn call_or_send(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<Delivery, FriendError> {
    let (frame, queued) = framed(host, to, body).await?;
    match call_frame(host, to, &frame).await {
        Ok(_) => Ok(Delivery::Receipted),
        Err(e) => {
            tracing::debug!(to = %to, kind = ?frame.type_id, error = %e,
                "not receipted by app_call — sending");
            deliver(host, to, &frame, &queued).await?;
            Ok(Delivery::Sent)
        }
    }
}

/// `app_call` `frame` to `to` and open the reply ([`send_to_peer_call`]).
async fn call_frame(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
) -> Result<Option<Opened>, FriendError> {
    let datagram = frame::wire(frame)?;
    let node = host.node()?;
    let target = route::cached_target(&node, to)
        .ok_or_else(|| FriendError::NotReady(format!("no cached route for peer {to}")))?;
    let reply = node
        .caller()
        .call_direct(&target, datagram)
        .await
        .map_err(|e| FriendError::Failed(format!("app_call: {e}")))?;
    if reply.is_empty() {
        return Ok(None);
    }
    if reply == crate::operations::direct::NAK {
        return Err(FriendError::Failed(format!("{to} refused the app_call")));
    }
    let opened = super::unwire(&reply)
        .ok_or_else(|| FriendError::Failed("app_call reply is not a 1:1 frame".into()))?;
    let opened = crate::operations::direct::frame::open(host, opened)
        .await
        .map_err(|_| FriendError::Failed("app_call reply does not open".into()))?;
    if opened.sender_hex != to {
        return Err(FriendError::Failed(format!(
            "app_call reply from {} for a call to {to}",
            opened.sender_hex
        )));
    }
    crate::operations::direct::frame::processed(host, &opened);
    Ok(Some(opened))
}

/// What one retry of a queued frame came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resend {
    /// It went out.
    Delivered,
    /// No route, or the send failed: try again next tick.
    Failed,
    /// It cannot be sent again (its kind is not one, its recipient is not a
    /// key, or the identity is gone): drop it.
    Unsendable,
}

/// Send a queued frame again over the peer's cached route, or their
/// mailbox's route when none is cached (the pending-message retry).
pub async fn resend(host: &dyn FriendHost, to: &str, queued: &Queued) -> Resend {
    let Ok(node) = host.node() else {
        return Resend::Failed;
    };
    let target = if let Some(target) = route::cached_target(&node, to) {
        target
    } else {
        let Some(blob) = route::fetch_from_mailbox(host, to).await else {
            return Resend::Failed;
        };
        let Some(target) = route::import(&node, to, &blob) else {
            return Resend::Failed;
        };
        tracing::debug!(to = %to, "discovered route via mailbox fallback");
        target
    };
    let datagram = match datagram_for_retry(host, to, queued) {
        Ok(datagram) => datagram,
        Err(error) => {
            tracing::warn!(to = %to, %error, "queued frame cannot be sent again — dropping");
            return Resend::Unsendable;
        }
    };
    match node.sender().send_direct(&target, datagram).await {
        Ok(()) => {
            tracing::debug!(to = %to, "queued frame delivered");
            Resend::Delivered
        }
        Err(error) => {
            if matches!(error, crate::error::TransportError::RouteUnusable { .. }) {
                route::invalidate(&node, to);
            }
            tracing::debug!(to = %to, %error, "queued frame retry failed");
            Resend::Failed
        }
    }
}

/// The datagram one retry sends.
fn datagram_for_retry(
    host: &dyn FriendHost,
    to: &str,
    queued: &Queued,
) -> Result<Vec<u8>, FriendError> {
    match queued {
        Queued::Wire { datagram } => Ok(datagram.clone()),
        Queued::Sealed {
            type_id,
            body,
            nonce,
        } => {
            let type_id = DmTypeId::from_u16(*type_id)
                .ok_or_else(|| FriendError::Invalid(format!("typeId {type_id:#06x}")))?;
            let body = DirectBody::decode(type_id, body)
                .map_err(|e| FriendError::Invalid(format!("queued body: {e}")))?;
            frame::wire(&frame::sealed_frame(host, to, &body, *nonce)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_queued_frame_survives_its_json_row() {
        for queued in [
            Queued::Wire {
                datagram: vec![b'D', 1, 2, 3],
            },
            Queued::Sealed {
                type_id: DmTypeId::FriendReject.as_u16(),
                body: Vec::new(),
                nonce: [9; 16],
            },
        ] {
            let row = serde_json::to_string(&queued).unwrap();
            assert_eq!(serde_json::from_str::<Queued>(&row).unwrap(), queued);
        }
    }

    #[test]
    fn hints_follow_the_kind() {
        assert_eq!(hint_for(DmTypeId::DmText), ContentHint::Resendable);
        assert_eq!(hint_for(DmTypeId::Typing), ContentHint::Implicit);
        assert_eq!(hint_for(DmTypeId::CallInvite), ContentHint::Default);
    }
}
