//! The one inbound 1:1 dispatch, on both hosts (plan E2.1, on `DmFrame`
//! since E2.3): a `'D'` datagram is opened ([`frame::open`]) and its body
//! handled here, or handed back as [`Routed::Unported`] when the step that
//! owns it has not come. The desktop handles those with its existing arms;
//! the daemon, which has none of those features, drops them.

pub(crate) mod admission;
pub mod frame;
mod handlers;
pub mod replay;
pub mod retry_requests;

pub use frame::{Dropped, Opened};
use rekindle_codec::capnp_codec::direct::DirectBody;

use crate::operations::dm;
use crate::operations::friend::FriendHost;
pub(crate) use crate::operations::prekeys::Rejection;

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
/// It is the one refusal of every `app_call` either host answers.
pub const NAK: &[u8] = b"N";

/// How long an inbound `app_call` may take to be answered before it is
/// left unanswered ([`Called::Unanswered`]; a deadline is no refusal), on
/// both hosts. Under Veilid's own wait for our answer, `rpc.timeout_ms`
/// (`CALL_TIMEOUT_MS`, 5 s: `rpc_app_call.rs:133-136` waits
/// `self.timeout`, set from `timeout_ms` at `rpc_processor/mod.rs:192`),
/// with a second for the reply to reach it, so ours fires first and the
/// Host stops working on a call whose answer Veilid would discard.
pub const CALL_DEADLINE: std::time::Duration =
    std::time::Duration::from_millis(rekindle_types::message::CALL_TIMEOUT_MS - 1_000);

/// What became of an inbound 1:1 `app_call`.
#[derive(Debug)]
pub enum Called {
    /// The reply: empty for a call that opened and was applied, or a
    /// duplicate of one (its return is the receipt); [`NAK`] for a judged
    /// refusal; or a frame.
    Answered(Vec<u8>),
    /// A kind whose owner step has not come; the desktop answers it.
    Unported(Box<Opened>),
    /// No answer: nothing was judged (the deadline passed, or an internal
    /// error of ours: our database, a store that failed, a reply that did
    /// not frame). A NAK is a refusal, so the host sends no reply; Veilid then
    /// sends nothing back (`rpc_app_call.rs:128-133`) and the caller's
    /// `app_call` times out, so its row is attempted again (RV2).
    Unanswered,
}

/// Handle one inbound `app_message`. `datagram` starts with the `'D'` tag;
/// anything else is dropped. A stream entry's message is a hint (plan
/// E2.5, D23): it is applied as it comes, and the sender's chain, which
/// is authoritative, is diffed for anything before it that was lost.
pub async fn receive(host: &dyn FriendHost, datagram: &[u8]) -> Routed {
    let Some(bytes) = dm::unwire(datagram) else {
        tracing::warn!(len = datagram.len(), "dropping untagged 1:1 message");
        return Routed::Handled;
    };
    let opened = match frame::open(host, bytes).await {
        Ok(opened) => opened,
        Err(dropped) => {
            by_message(host, dropped);
            return Routed::Handled;
        }
    };
    if opened.stream_seq > 0 {
        dm::stream::on_hint(host, &opened.sender_hex);
    }
    handlers::handle(host, opened).await
}

/// A frame that did not open by message: a rejected init's
/// `PreKeyRejected` goes back by message (B5).
fn by_message(host: &dyn FriendHost, dropped: Dropped) {
    if let Dropped::Rejected { to, frame } = dropped {
        crate::operations::prekeys::send_rejection(host, &to, *frame);
    }
}

/// The reply to a first contact we called `to` with, other than the
/// receipt: their `PreKeyRejected` (B5, finding 46) is handled as one
/// that came by message (`prekeys::on_rejected`, the one resend). A reply
/// that did not open, or a rejection naming no unanswered init of ours, is
/// [`Rejection::NotOurs`], a refusal (finding 46 refined); one we could
/// not judge or apply (our own failure) is [`Rejection::Failed`] (R5).
pub(crate) async fn on_first_contact_reply(
    host: &dyn FriendHost,
    to: &str,
    reply: &[u8],
) -> Rejection {
    let Some(bytes) = dm::unwire(reply) else {
        return Rejection::NotOurs;
    };
    let opened = match frame::open(host, bytes).await {
        Ok(opened) => opened,
        Err(Dropped::Failed) => return Rejection::Failed,
        Err(_) => return Rejection::NotOurs,
    };
    if opened.sender_hex != to {
        tracing::warn!(to = %to, from = %opened.sender_hex, "dropping a call reply by another sender");
        frame::processed(host, &opened);
        return Rejection::NotOurs;
    }
    if !matches!(opened.body, DirectBody::PreKeyRejected(_)) {
        frame::processed(host, &opened);
        return Rejection::NotOurs;
    }
    match handlers::apply(host, opened).await {
        handlers::Applied::Done(true) => Rejection::Applied,
        handlers::Applied::Done(false) => Rejection::Failed,
        handlers::Applied::Refused | handlers::Applied::Unported(_) => Rejection::NotOurs,
    }
}

/// Handle one entry of `writer`'s DM chain (a wired frame they appended):
/// the receive path of [`receive`], for a frame by `writer` only.
pub async fn receive_entry(host: &dyn FriendHost, writer: &str, datagram: &[u8]) -> Routed {
    let Some(bytes) = dm::unwire(datagram) else {
        tracing::warn!(from = %writer, "dropping an untagged chain entry");
        return Routed::Handled;
    };
    let opened = match frame::open(host, bytes).await {
        Ok(opened) => opened,
        Err(dropped) => {
            by_message(host, dropped);
            return Routed::Handled;
        }
    };
    if opened.sender_hex != writer {
        tracing::warn!(from = %writer, by = %opened.sender_hex, "dropping a chain entry by another sender");
        frame::processed(host, &opened);
        return Routed::Handled;
    }
    handlers::handle(host, opened).await
}

/// Handle one inbound `app_call`. A relay offer is unported (the desktop
/// replies with its ack). A refused frame is answered [`NAK`], one not
/// judged (an internal error of ours, a store that failed) is left
/// unanswered, and a copy
/// of one that opened the empty receipt (plan audit G3). A sealed kind
/// advances no ratchet and leaves no `unprocessed` row, so it is applied
/// before the answer, and only a stored one is receipted (G4). A ratchet
/// kind is answered once open — its step and `unprocessed` row are
/// committed — and applied after (D24: the ack follows the commit, the
/// apply follows the ack, the replay covers a crash between).
pub async fn receive_call(host: &dyn FriendHost, datagram: &[u8]) -> Called {
    let Some(bytes) = dm::unwire(datagram) else {
        tracing::warn!(len = datagram.len(), "dropping untagged 1:1 call");
        return Called::Answered(NAK.to_vec());
    };
    let opened = match frame::open(host, bytes).await {
        Ok(opened) => opened,
        Err(Dropped::Duplicate) => return Called::Answered(Vec::new()),
        Err(Dropped::Refused) => return Called::Answered(NAK.to_vec()),
        Err(Dropped::Failed) => return Called::Unanswered,
        // B5 by the call itself (finding 46): the caller tells a refused
        // prekey (this reply, its one resend) from any other refusal (NAK).
        Err(Dropped::Rejected { to, frame }) => {
            return match crate::operations::prekeys::rejection_reply(&to, &frame) {
                Some(reply) => Called::Answered(reply),
                None => Called::Unanswered,
            };
        }
    };
    if matches!(opened.body, DirectBody::RelayOffer(_)) {
        frame::processed(host, &opened);
        return Called::Unported(Box::new(opened));
    }
    if opened.unprocessed.is_none() {
        return match handlers::apply(host, opened).await {
            handlers::Applied::Done(true) => Called::Answered(Vec::new()),
            // Its store failed: ours, not a refusal (RV5 R2).
            handlers::Applied::Done(false) => Called::Unanswered,
            handlers::Applied::Refused => Called::Answered(NAK.to_vec()),
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

/// [`receive_call`] within [`CALL_DEADLINE`]: past it the call is
/// [`Called::Unanswered`] (a deadline is no refusal). Each host answers its
/// 1:1 `app_call`s through this.
pub async fn answer_call(host: &dyn FriendHost, datagram: &[u8]) -> Called {
    tokio::time::timeout(CALL_DEADLINE, receive_call(host, datagram))
        .await
        .unwrap_or_else(|_| {
            tracing::warn!("1:1 app_call exceeded its deadline — left unanswered");
            Called::Unanswered
        })
}

/// Replay what was decrypted but never applied (a crash between the
/// ratchet commit and the apply, D24): a Host unlock stage on both hosts.
/// Each row is judged by the gate again (a block or a removal since the
/// decrypt refuses it, and the row goes). An unported kind's replay is
/// dropped: its owner's live state did not survive the restart either.
/// The send log and the retry state past their horizons are pruned first
/// (plan E2.5, D24).
pub async fn replay_unprocessed(host: &dyn FriendHost) {
    retry_requests::prune(host).await;
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
