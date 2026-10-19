//! The one inbound 1:1 dispatch, on both hosts (plan E2.1, on `DmFrame`
//! since E2.3): a `'D'` datagram is opened ([`frame::open`]) and its body
//! handled here, or handed back as [`Routed::Unported`] when the step that
//! owns it has not come. The desktop handles those with its existing arms;
//! the daemon, which has none of those features, drops them.

pub mod frame;
mod handlers;
pub mod replay;

pub use frame::Opened;
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

/// What became of an inbound 1:1 `app_call`.
#[derive(Debug)]
pub enum Called {
    /// The reply: empty for a handled call (its return is the receipt), or
    /// a frame.
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
    let Some(opened) = frame::open(host, bytes).await else {
        return Routed::Handled;
    };
    handlers::handle(host, opened).await
}

/// Handle one inbound `app_call`: a friend request is applied and answered
/// with its receipt; a relay offer is unported (the desktop replies with
/// its ack). Anything else is applied as an `app_message` would be.
pub async fn receive_call(host: &dyn FriendHost, datagram: &[u8]) -> Called {
    let Some(bytes) = dm::unwire(datagram) else {
        tracing::debug!(len = datagram.len(), "dropping untagged 1:1 call");
        return Called::Answered(Vec::new());
    };
    let Some(opened) = frame::open(host, bytes).await else {
        return Called::Answered(Vec::new());
    };
    match handlers::handle(host, opened).await {
        Routed::Handled => Called::Answered(Vec::new()),
        Routed::Unported(opened) if matches!(opened.body, DirectBody::RelayOffer(_)) => {
            Called::Unported(opened)
        }
        Routed::Unported(opened) => {
            tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
                "unported kind by app_call — no reply");
            Called::Answered(Vec::new())
        }
    }
}

/// Replay what was decrypted but never applied (a crash between the
/// ratchet commit and the apply, D24): a Host unlock stage on both hosts.
/// An unported kind's replay is dropped: its owner's live state did not
/// survive the restart either.
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
            continue;
        };
        if let Routed::Unported(opened) = handlers::handle(host, opened).await {
            tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
                "dropping an unported kind on replay");
        }
    }
}
