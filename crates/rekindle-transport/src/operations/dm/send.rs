//! Sending one 1:1 frame: the cached route, an inline route refresh from
//! the peer's profile and a status probe (§13.5). Nothing here queues: a
//! send that must arrive is an outbox row (plan E2.5, D22: `dm::outbox`,
//! `dm::handshake`), and a send that finds no route fails visibly. Every send goes through the node's ports; the
//! bytes are [`frame::wire`]. The Strand Relay fallback (§13.3) returns at
//! E5.5, whose pool entries name the relay's identity: a relay frame is
//! addressed and signed to it, and a bare route blob names no one.
//!
//! A `friend_handshake` row keeps its frame as [`Framed`]: a ratchet or
//! PreKeyInit frame's wire bytes exactly as encrypted and recorded (sent
//! again byte-identical: the ratchet state was committed before the first
//! attempt, and the receiver drops a used `(dh, n)`), and a sealed frame's
//! body and nonce, re-sealed with a fresh `sent_at` per attempt (the replay
//! guard drops a copy that already arrived).

use rekindle_codec::capnp_codec::direct::{Carrier, ContentHint, DirectBody, DmFrame, DmTypeId};
use serde::{Deserialize, Serialize};

use super::frame::{self, Nonce};
use super::route;
use crate::operations::direct::frame::Opened;
use crate::operations::friend::{FriendError, FriendHost};

/// A frame as a `friend_handshake` row stores it (JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Framed {
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
    /// An invite's redemption (plan E2.5, D17): the sealed `FriendRequest`
    /// frame, written into the invite's request inbox as its one-time
    /// writer (`friend::inbox`). The writer's secret is stored in the row
    /// only because the row is in the identity's SQLCipher database (ruling
    /// R2) and is deleted when the row settles.
    Inbox {
        record_key: String,
        writer: String,
        #[serde(with = "rekindle_types::base64_bytes")]
        frame: Vec<u8>,
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

/// The frame to `to` for `body`, and how an outbox row keeps it.
pub(super) async fn framed(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<(DmFrame, Framed), FriendError> {
    let type_id = body.type_id();
    if !crate::operations::friend::verification::is_protective(type_id) {
        crate::operations::friend::verification::require_acknowledged(host, to).await?;
    }
    if type_id.carrier() == Carrier::Sealed {
        let mut nonce = Nonce::default();
        host.rng().fill(&mut nonce);
        let frame = frame::sealed_frame(host, to, body, nonce)?;
        let queued = Framed::Sealed {
            type_id: type_id.as_u16(),
            body: body.encode(),
            nonce,
        };
        return Ok((frame, queued));
    }
    let frame = frame::frame(host, to, body, hint_for(type_id)).await?;
    let queued = Framed::Wire {
        datagram: frame::wire(&frame)?,
    };
    Ok((frame, queued))
}

/// Send `body` to `to` once, as its kind travels: the cached route, then
/// one refreshed inline. An ephemeral kind (typing, a call signal, DM
/// video: `DmTypeId::is_ephemeral`) that finds no route is dropped:
/// minutes late it is worse than none.
///
/// # Errors
/// The frame could not be sealed (no session for a ratchet kind), it is
/// over Veilid's message cap (plan C7.25), or a kind that is not ephemeral
/// found no usable route (`NotReady`; what must arrive is an outbox row).
pub async fn send_to_peer(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
) -> Result<(), FriendError> {
    let (frame, _) = framed(host, to, body).await?;
    deliver(host, to, &frame).await
}

/// Send a frame built elsewhere (a resend a retry request asked for) to
/// `to` once, like [`send_to_peer`].
///
/// # Errors
/// The frame is over Veilid's message cap, or no usable route took it.
pub async fn send_frame(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
) -> Result<(), FriendError> {
    deliver(host, to, frame).await
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
    route_send_datagram(host, to, &frame::wire(frame)?, frame.type_id, refresh).await
}

/// [`route_send`] of a datagram already wired (a `friend_handshake` row's).
///
/// # Errors
/// The node is not up.
pub(super) async fn route_send_datagram(
    host: &dyn FriendHost,
    to: &str,
    datagram: &[u8],
    kind: DmTypeId,
    refresh: bool,
) -> Result<bool, FriendError> {
    let datagram = datagram.to_vec();
    let node = host.node()?;
    let failure = match route::cached_target(&node, to) {
        Some(target) => match node.sender().send_direct(&target, datagram.clone()).await {
            Ok(()) => {
                tracing::debug!(to = %to, ?kind, "frame sent via veilid");
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
/// E2.5): a ratchet message carrying the seq the peer's `DmAck` names and
/// `sent_at_ms`, when the entry was written (E2.B F3),
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
    sent_at_ms: u64,
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
    frame::ratchet_frame_at(host, to, body, hint_for(type_id), stream_seq, sent_at_ms).await
}

/// Send `body` to `to` once: a receipt the peer's resend
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

/// One send of `frame`; an ephemeral kind that finds no route is dropped.
async fn deliver(host: &dyn FriendHost, to: &str, frame: &DmFrame) -> Result<(), FriendError> {
    let ephemeral = frame.type_id.is_ephemeral();
    if route_send(host, to, frame, !ephemeral).await? || ephemeral {
        return Ok(());
    }
    Err(FriendError::NotReady(format!(
        "no usable route to {to} for {:?}",
        frame.type_id
    )))
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

/// `app_call` `frame` to `to` and open the reply ([`send_to_peer_call`]).
async fn call_frame(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
) -> Result<Option<Opened>, FriendError> {
    call_datagram(host, to, frame::wire(frame)?).await
}

/// [`call_frame`] of a datagram already wired (a `friend_handshake` row's).
///
/// # Errors
/// As [`send_to_peer_call`].
pub(super) async fn call_datagram(
    host: &dyn FriendHost,
    to: &str,
    datagram: Vec<u8>,
) -> Result<Option<Opened>, FriendError> {
    let node = host.node()?;
    let target = if let Some(target) = route::cached_target(&node, to) {
        target
    } else {
        let blob = route::fetch_from_profile(host, to)
            .await
            .ok_or_else(|| FriendError::NotReady(format!("no route for peer {to}")))?;
        route::import(&node, to, &blob)
            .ok_or_else(|| FriendError::NotReady(format!("unusable route for peer {to}")))?
    };
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

/// The datagram one attempt of a stored frame sends, and its kind.
///
/// # Errors
/// The stored frame does not decode, or the sealed kind does not re-seal.
pub(super) fn datagram_of(
    host: &dyn FriendHost,
    to: &str,
    framed: &Framed,
) -> Result<(Vec<u8>, DmTypeId), FriendError> {
    match framed {
        Framed::Wire { datagram } => {
            let frame = super::unwire(datagram)
                .and_then(|bytes| DmFrame::decode(bytes).ok())
                .ok_or_else(|| FriendError::Invalid("stored frame does not decode".into()))?;
            Ok((datagram.clone(), frame.type_id))
        }
        Framed::Sealed {
            type_id,
            body,
            nonce,
        } => {
            let type_id = DmTypeId::from_u16(*type_id)
                .ok_or_else(|| FriendError::Invalid(format!("typeId {type_id:#06x}")))?;
            let body = DirectBody::decode(type_id, body)
                .map_err(|e| FriendError::Invalid(format!("queued body: {e}")))?;
            let datagram = frame::wire(&frame::sealed_frame(host, to, &body, *nonce)?)?;
            Ok((datagram, type_id))
        }
        Framed::Inbox { .. } => Err(FriendError::Invalid(
            "an inbox write is not sent over a route".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_frame_survives_its_json_row() {
        for queued in [
            Framed::Wire {
                datagram: vec![b'D', 1, 2, 3],
            },
            Framed::Sealed {
                type_id: DmTypeId::FriendReject.as_u16(),
                body: Vec::new(),
                nonce: [9; 16],
            },
        ] {
            let row = serde_json::to_string(&queued).unwrap();
            assert_eq!(serde_json::from_str::<Framed>(&row).unwrap(), queued);
        }
    }

    #[test]
    fn hints_follow_the_kind() {
        assert_eq!(hint_for(DmTypeId::DmText), ContentHint::Resendable);
        assert_eq!(hint_for(DmTypeId::Typing), ContentHint::Implicit);
        assert_eq!(hint_for(DmTypeId::CallInvite), ContentHint::Default);
    }
}
