//! The one inbound 1:1 dispatch, on both hosts (plan E2.1, on `DmFrame`
//! since E2.3): a `'D'` datagram is opened ([`frame::open`]) and its body
//! handled here, or handed back as [`Routed::Unported`] when the step that
//! owns it has not come. The desktop handles those with its existing arms;
//! the daemon, which has none of those features, drops them.

pub(crate) mod admission;
pub mod frame;
mod handlers;
pub mod held;
pub mod replay;
pub mod retry_requests;
mod sealed_open;

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

/// The `app_call` reply to a call we could not judge or apply (review RV5
/// M2): our own fault (our database, no unlocked identity, a store that
/// failed on our side, a copy whose first is still in flight, our
/// deadline). One byte, distinct from the receipt (empty), [`NAK`] and a
/// `'D'` frame. Answering at once frees the Veilid RPC worker an
/// unanswered call holds for `rpc.timeout_ms` (0.5.8 `rpc_app_call.rs:119-139`);
/// the caller attempts its row again, as on a timeout, with no route
/// refresh (the call reached us).
pub const RETRY_LATER: &[u8] = rekindle_types::message::CALL_RETRY_LATER;

/// How long an inbound `app_call`'s reply may take before it is answered
/// [`RETRY_LATER`] ([`Called::RetryLater`]; a deadline is no refusal), on
/// both hosts. The work goes on to completion past it (review RV5 H1). Under Veilid's own wait for our answer, `rpc.timeout_ms`
/// (`CALL_TIMEOUT_MS`, 5 s: `rpc_app_call.rs:129-133` waits
/// `self.timeout`, set from `timeout_ms` at `rpc_processor/mod.rs:222-223`),
/// with a second for the reply to reach it, so ours fires first and the
/// Host stops working on a call whose answer Veilid would discard.
pub const CALL_DEADLINE: std::time::Duration =
    std::time::Duration::from_millis(rekindle_types::message::CALL_TIMEOUT_MS - 1_000);

/// What became of an inbound 1:1 `app_call`.
#[derive(Debug)]
pub enum Called {
    /// The reply: empty for a call that opened and was applied, or a
    /// duplicate of one (its return is the receipt); [`NAK`] for a judged
    /// refusal (an unported kind among them: no handler is a judgement that
    /// this host takes no such call, final for the caller as any refusal,
    /// review RV5 L1); or a frame.
    Answered(Vec<u8>),
    /// A kind whose owner step has not come; the desktop answers it.
    Unported(Box<Opened>),
    /// Nothing was judged (the deadline passed, a copy whose first is in
    /// flight, or an internal error of ours: our database, a store that
    /// failed on our side, a reply that did not frame). A NAK is a
    /// refusal, so the host answers [`RETRY_LATER`] (review RV5 M2) and the
    /// caller attempts its row again (RV2).
    RetryLater,
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
    let digest = dm::ack::digest(bytes);
    let opened = match frame::open(host, bytes).await {
        Ok(opened) => opened,
        Err(dropped) => {
            by_message(host, dropped, &digest);
            return Routed::Handled;
        }
    };
    // Held: neither acknowledged nor remembered for a copy's re-ack.
    if opened.held {
        held::keep(host, opened).await;
        return Routed::Handled;
    }
    dm::ack::opened(host, &opened, digest);
    if opened.stream_seq > 0 {
        dm::stream::on_hint(host, &opened.sender_hex);
    }
    handlers::handle(host, opened).await
}

/// A frame that did not open by message or from a chain: a rejected
/// init's `PreKeyRejected` goes back by message (B5), and an exact copy
/// (`digest`, [`dm::ack::digest`]) of an entry we applied is acknowledged
/// again by its own seq (Briar answers every duplicate; live A/B finding
/// 73b).
fn by_message(host: &dyn FriendHost, dropped: Dropped, digest: &[u8; 32]) {
    match dropped {
        Dropped::Rejected { to, frame } => {
            crate::operations::prekeys::send_rejection(host, &to, *frame);
        }
        Dropped::Duplicate { from } => dm::ack::duplicate(host, &from, digest),
        // A blocked sender is never acked (LV23 review H1/H2); an in-flight
        // copy's first copy settles the receipt.
        Dropped::Blocked
        | Dropped::InFlight
        | Dropped::Refused
        | Dropped::NotYet
        | Dropped::Failed => {}
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
        Err(Dropped::Failed | Dropped::InFlight) => return Rejection::Failed,
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

/// What became of one entry of a writer's chain ([`receive_entry`]).
pub enum EntryFate {
    /// Its content is received, decrypted and stored, or it was judged (a
    /// duplicate of one opened, a refusal, junk): the cursor may pass it.
    Held,
    /// Its content is not held yet: its first copy is still being applied,
    /// our own step failed, its store failed (its `unprocessed` row kept
    /// for the replay), or it waits on our friendship's accept. The cursor,
    /// which a `DmAck` reports, stays below it (live finding L-ACK1).
    Pending,
    /// A kind whose owner step has not come, handed back.
    Unported(Box<Opened>),
}

/// Receive one entry of `writer`'s chain (`datagram`, a wired frame) by
/// the one 1:1 path: opened, judged and applied as a hint would be.
pub async fn receive_entry(host: &dyn FriendHost, writer: &str, datagram: &[u8]) -> EntryFate {
    let Some(bytes) = dm::unwire(datagram) else {
        tracing::warn!(from = %writer, "dropping an untagged chain entry");
        return EntryFate::Held;
    };
    let digest = dm::ack::digest(bytes);
    let opened = match frame::open(host, bytes).await {
        Ok(opened) => opened,
        Err(dropped @ (Dropped::InFlight | Dropped::Failed | Dropped::NotYet)) => {
            by_message(host, dropped, &digest);
            return EntryFate::Pending;
        }
        Err(dropped) => {
            by_message(host, dropped, &digest);
            return EntryFate::Held;
        }
    };
    if opened.held && opened.sender_hex == writer {
        held::keep(host, opened).await;
        return EntryFate::Pending;
    }
    dm::ack::opened(host, &opened, digest);
    if opened.sender_hex != writer {
        tracing::warn!(from = %writer, by = %opened.sender_hex, "dropping a chain entry by another sender");
        frame::processed(host, &opened);
        return EntryFate::Held;
    }
    match handlers::apply(host, opened).await {
        handlers::Applied::Done(true) | handlers::Applied::Refused => EntryFate::Held,
        handlers::Applied::Done(false) => EntryFate::Pending,
        handlers::Applied::Unported(opened) => EntryFate::Unported(opened),
    }
}

/// Handle one inbound `app_call`. A relay offer is unported (the desktop
/// replies with its ack). A blocked sender's call gets the empty receipt
/// and is dropped unstored (LV23 review). A refused frame is answered [`NAK`], one not
/// judged (an internal error of ours, a store that failed on our side, a
/// copy whose first is in flight) or refused only until the accept of our
/// request to its sender lands ([`Dropped::NotYet`]) [`RETRY_LATER`], and a copy
/// of one that opened the empty receipt (plan audit G3). A sealed kind
/// advances no ratchet and leaves no `unprocessed` row, so it is applied
/// before the answer, and only a stored one is receipted (G4). A ratchet
/// kind is answered once open — its step and `unprocessed` row are
/// committed — and applied after (D24: the ack follows the commit, the
/// apply follows the ack, the replay covers a crash between). A copy of a
/// call we receipted, from a friend or open-handshake peer over our live
/// General route `route`, is evidence our receipt was lost on it (plan
/// RV2, [`re_sent`]).
pub async fn receive_call(host: &dyn FriendHost, datagram: &[u8], route: Option<&str>) -> Called {
    let Some(bytes) = dm::unwire(datagram) else {
        tracing::warn!(len = datagram.len(), "dropping untagged 1:1 call");
        return Called::Answered(NAK.to_vec());
    };
    let opened = match frame::open(host, bytes).await {
        Ok(opened) => opened,
        // The silent receipt: a block is never told (LV23 review H1/H2).
        Err(Dropped::Duplicate { from }) => {
            re_sent(host, &from, bytes, route).await;
            return Called::Answered(Vec::new());
        }
        Err(Dropped::Blocked) => return Called::Answered(Vec::new()),
        Err(Dropped::Refused) => return Called::Answered(NAK.to_vec()),
        // Not judged, or refused only until the accept of our request
        // lands (finding GDI1): the caller attempts it again.
        Err(Dropped::Failed | Dropped::InFlight | Dropped::NotYet) => return Called::RetryLater,
        // B5 by the call itself (finding 46): the caller tells a refused
        // prekey (this reply, its one resend) from any other refusal (NAK).
        Err(Dropped::Rejected { to, frame }) => {
            return match crate::operations::prekeys::rejection_reply(&to, &frame) {
                Some(reply) => Called::Answered(reply),
                None => Called::RetryLater,
            };
        }
    };
    // Held: its row is committed, so receipted; applied at the accept.
    if opened.held {
        held::keep(host, opened).await;
        return Called::Answered(Vec::new());
    }
    // The unported kinds that come by call: a relay offer (its owner
    // replies with its ack) and a call's `CallEnd` (plan CE4: receipted,
    // its owner applies it after answering the receipt).
    if matches!(
        opened.body,
        DirectBody::RelayOffer(_) | DirectBody::CallEnd(_)
    ) {
        frame::processed(host, &opened);
        return Called::Unported(Box::new(opened));
    }
    if opened.unprocessed.is_none() {
        return match handlers::apply(host, opened).await {
            handlers::Applied::Done(true) => Called::Answered(Vec::new()),
            // Its store failed on our side: not a refusal (RV5 R2, M2).
            handlers::Applied::Done(false) => Called::RetryLater,
            handlers::Applied::Refused => Called::Answered(NAK.to_vec()),
            // No handler: judged (review RV5 L1, [`Called::Answered`]).
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

/// [`receive_call`], its reply raced against [`CALL_DEADLINE`]: past it
/// the call is [`Called::RetryLater`] (a deadline is no refusal). The work
/// itself runs on the session scope to completion (review RV5 H1), so a
/// half-done receive is never dropped at an await: a sealed frame's nonce
/// settles when its store commits or is forgotten when it fails, and its
/// retry is judged on that. Each host answers its 1:1 `app_call`s through
/// this.
pub async fn answer_call(host: &dyn FriendHost, datagram: &[u8], route: Option<&str>) -> Called {
    let (answer, answered) = tokio::sync::oneshot::channel();
    let (owned, datagram) = (host.owned(), datagram.to_vec());
    let route = route.map(str::to_owned);
    host.scope().spawn_or_drop("answer 1:1 call", async move {
        // Past the deadline the answer has no taker; the work is done.
        let _ = answer.send(receive_call(owned.as_ref(), &datagram, route.as_deref()).await);
    });
    match tokio::time::timeout(CALL_DEADLINE, answered).await {
        Ok(Ok(called)) => called,
        Ok(Err(_)) => {
            tracing::debug!("1:1 app_call dropped with its session — retry later");
            Called::RetryLater
        }
        Err(_) => {
            tracing::warn!("1:1 app_call exceeded its deadline — answered retry later");
            Called::RetryLater
        }
    }
}

/// `from` re-sent `frame`, a call we receipted, over our route `route`
/// (plan RV2): the copy is authenticated (a sealed frame's signature is
/// verified before the replay store sees it, a ratchet copy needs the
/// session's AEAD: [`frame::open`]), so it counts against our live General
/// route when `from` is a friend or a peer with an open handshake.
async fn re_sent(host: &dyn FriendHost, from: &str, frame: &[u8], route: Option<&str>) {
    let Ok(node) = host.node() else {
        return;
    };
    let live = node.liveness();
    if !live.over_general(route) || !host.open_handshake_or_friend(from).await {
        return;
    }
    live.duplicate_call(from, dm::ack::digest(frame), route);
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
        // Judged again: the sender may have been blocked or removed since,
        // or still be one we asked (held again, plan E2.5i R2-2).
        match held::judge_replay(host, &opened).await {
            held::Replay::Apply => {}
            held::Replay::Hold => {
                held::keep(host, opened).await;
                continue;
            }
            held::Replay::Refuse => {
                frame::processed(host, &opened);
                continue;
            }
        }
        if let Routed::Unported(opened) = handlers::handle(host, opened).await {
            tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
                "dropping an unported kind on replay");
        }
    }
}
