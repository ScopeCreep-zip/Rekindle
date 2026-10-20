//! The one inbound 1:1 dispatch, on both hosts (plan E2.1, on `DmFrame`
//! since E2.3): a `'D'` datagram is opened ([`frame::open`]) and its body
//! handled here, or handed back as [`Routed::Unported`] when the step that
//! owns it has not come. The desktop handles those with its existing arms;
//! the daemon, which has none of those features, drops them.

pub(crate) mod admission;
pub mod frame;
mod handlers;
pub mod replay;

pub use frame::{Dropped, Opened};
use rekindle_codec::capnp_codec::direct::DirectBody;

use crate::operations::dm;
use crate::operations::friend::FriendHost;

/// What became of an inbound 1:1 message.
#[derive(Debug)]
pub enum Routed {
    /// Handled here, or dropped (logged).
    Handled,
    /// A kind whose owner step has not come: DM video (E4.3.8), calls and
    /// group calls (E4.1), group-DM invites and leaves (E2.6), relay
    /// offers, envelopes and status requests and push relay (E5.2). Its `unprocessed` row is already gone: the
    /// hand-off is its apply.
    Unported(Box<Opened>),
}

/// The `app_call` reply that refuses a call: no receipt (plan audit G3).
/// One byte, so it is never empty (the receipt) nor a `'D'` frame.
pub const NAK: &[u8] = b"N";

/// What became of an inbound 1:1 `app_call`.
#[derive(Debug)]
pub enum Called {
    /// The reply: empty for a call that opened and was applied, or a
    /// duplicate of one (its return is the receipt); [`NAK`] for any drop;
    /// or a frame.
    Answered(Vec<u8>),
    /// A kind whose owner step has not come; the desktop answers it.
    Unported(Box<Opened>),
}

/// Handle one inbound `app_message`. `datagram` starts with the `'D'` tag;
/// anything else is dropped.
pub async fn receive(host: &dyn FriendHost, datagram: &[u8]) -> Routed {
    let Some(bytes) = dm::unwire(datagram) else {
        tracing::debug!(len = datagram.len(), "dropping untagged 1:1 message");
        return Routed::Handled;
    };
    let Ok(opened) = frame::open(host, bytes).await else {
        return Routed::Handled;
    };
    handlers::handle(host, opened).await
}

/// Handle one inbound `app_call`. A relay offer is unported (the desktop
/// replies with its ack). A dropped frame is answered [`NAK`], and a copy
/// of one that opened the empty receipt (plan audit G3). A sealed kind
/// advances no ratchet and leaves no `unprocessed` row, so it is applied
/// before the answer, and only a stored one is receipted (G4). A ratchet
/// kind is answered once open — its step and `unprocessed` row are
/// committed — and applied after (D24: the ack follows the commit, the
/// apply follows the ack, the replay covers a crash between).
pub async fn receive_call(host: &dyn FriendHost, datagram: &[u8]) -> Called {
    let Some(bytes) = dm::unwire(datagram) else {
        tracing::debug!(len = datagram.len(), "dropping untagged 1:1 call");
        return Called::Answered(NAK.to_vec());
    };
    let opened = match frame::open(host, bytes).await {
        Ok(opened) => opened,
        Err(Dropped::Duplicate) => return Called::Answered(Vec::new()),
        Err(Dropped::Refused) => return Called::Answered(NAK.to_vec()),
    };
    if matches!(opened.body, DirectBody::RelayOffer(_)) {
        frame::processed(host, &opened);
        return Called::Unported(Box::new(opened));
    }
    if opened.unprocessed.is_none() {
        return match handlers::apply(host, opened).await {
            handlers::Applied::Done(true) => Called::Answered(Vec::new()),
            handlers::Applied::Done(false) => Called::Answered(NAK.to_vec()),
            handlers::Applied::Unported(opened) => {
                tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
                    "unported kind by app_call — dropped");
                Called::Answered(NAK.to_vec())
            }
        };
    }
    let owned = host.owned();
    host.scope().spawn_or_drop("apply 1:1 call", async move {
        if let Routed::Unported(opened) = handlers::handle(owned.as_ref(), opened).await {
            tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
                "unported kind by app_call — dropped");
        }
    });
    Called::Answered(Vec::new())
}

/// Replay what was decrypted but never applied (a crash between the
/// ratchet commit and the apply, D24): a Host unlock stage on both hosts.
/// Each row is judged by the gate again (a block or a removal since the
/// decrypt refuses it, and the row goes). An unported kind's replay is
/// dropped: its owner's live state did not survive the restart either.
pub async fn replay_unprocessed(host: &dyn FriendHost) {
    let (Ok(local), Ok(db)) = (host.local_party(), host.db()) else {
        return;
    };
    let owner = local.owner();
    let now = local.now_ms;
    let rows = match db
        .call(move |conn| rekindle_db::repo::unprocessed::take_for_replay(conn, &owner, now))
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "unprocessed messages unreadable; replay skipped");
            return;
        }
    };
    for row in rows {
        let Some(opened) = frame::replayed(&row) else {
            frame::delete_unprocessed(host, row.key);
            continue;
        };
        // Judged again: the sender may have been blocked or removed since.
        if !admission::admitted(host, &opened.sender_hex, opened.type_id).await {
            frame::processed(host, &opened);
            continue;
        }
        if let Routed::Unported(opened) = handlers::handle(host, opened).await {
            tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
                "dropping an unported kind on replay");
        }
    }
}
