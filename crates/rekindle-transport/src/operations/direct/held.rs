//! Messages held for a friendship we asked for (plan E2.5i, live A/B
//! finding R2-2): the peer's accept and their first message travel apart,
//! so the message can arrive first, while we still hold our request as
//! `PendingOut`. *Decided from research 2026-11-15* (Signal ignores a 1:1
//! message only from a blocked sender, `MessageContentProcessor.kt:190-208`;
//! Session takes an incoming message as the approval,
//! `conversation.ts:1250-1255`, a trust shortcut we refuse): such a message
//! is held, never applied and never acknowledged, until the accept commits
//! ([`release`]), and then applied as it would have been on arrival.
//!
//! - Only user content that decrypted on a session with the sender is held
//!   ([`holds`]): a ratchet message or a PreKeyInit, so the peer's identity
//!   key authenticated it (PQXDH). A sealed frame is never held.
//! - A blocked sender is dropped before this ([`super::admission::unblocked`]).
//! - The message's `unprocessed` row is the durable copy (D24): held rows
//!   survive a restart, and the unlock replay holds them again
//!   ([`judge_replay`]).
//! - Nothing approves the friendship: the accept alone commits it.
//! - Held means not acknowledged: the opened frame is not remembered for
//!   [`crate::operations::dm::ack::duplicate`], so a copy of it draws no
//!   ack; the ack follows the apply, and the sender's re-ask covers a held
//!   message dropped at [`MAX_HELD_PER_PEER`] (its re-hint is framed
//!   afresh, so it opens as a new message).

use std::collections::{HashMap, VecDeque};

use rekindle_codec::capnp_codec::direct::{DmTypeId, MAX_DM_ACK_SEQS};
use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rekindle_types::friend::FriendshipState;

use super::admission::{self, Status};
use super::frame::{self, Opened};
use super::{handlers, Routed};
use crate::operations::friend::{state, FriendHost};

/// Most messages held per peer: what one `DmAck` names
/// ([`MAX_DM_ACK_SEQS`]), so the ack that follows the release covers every
/// held message by its seq. A message past it is dropped unacknowledged and
/// the sender's re-ask delivers it after the accept.
pub const MAX_HELD_PER_PEER: usize = MAX_DM_ACK_SEQS;

/// How long a message is held, milliseconds: the sender's outbox retries
/// both its accept (`friend_handshake`) and its `dm` rows until
/// [`OUTBOX_DEADLINE`], so an accept may still land that late, and a hold
/// longer than that waits for an accept nobody will send.
pub const HELD_EXPIRY_MS: u64 = DELIVERY_HORIZON.as_secs() * 1_000;

/// The messages held per peer (hex), oldest first, each with when it was
/// held.
#[derive(Debug, Default)]
pub struct HeldMessages {
    by_peer: HashMap<String, VecDeque<(Opened, u64)>>,
}

impl HeldMessages {
    /// Hold `opened` from its sender at `now_ms`; handed back when the
    /// sender already holds [`MAX_HELD_PER_PEER`].
    pub(crate) fn hold(&mut self, opened: Opened, now_ms: u64) -> Result<(), Box<Opened>> {
        let queue = self.by_peer.entry(opened.sender_hex.clone()).or_default();
        if queue.len() >= MAX_HELD_PER_PEER {
            return Err(Box::new(opened));
        }
        queue.push_back((opened, now_ms));
        Ok(())
    }

    /// Take `peer`'s held messages: those held within [`HELD_EXPIRY_MS`] of
    /// `now_ms`, oldest first, and the expired ones.
    pub(crate) fn take(&mut self, peer: &str, now_ms: u64) -> (Vec<Opened>, Vec<Opened>) {
        let queue = self.by_peer.remove(peer).unwrap_or_default();
        let (live, expired): (Vec<_>, Vec<_>) = queue
            .into_iter()
            .partition(|(_, at)| now_ms.saturating_sub(*at) < HELD_EXPIRY_MS);
        (
            live.into_iter().map(|(o, _)| o).collect(),
            expired.into_iter().map(|(o, _)| o).collect(),
        )
    }

    /// Forget everything (lock); the rows stay for the next unlock.
    pub fn clear(&mut self) {
        self.by_peer.clear();
    }

    #[cfg(test)]
    fn len(&self, peer: &str) -> usize {
        self.by_peer.get(peer).map_or(0, VecDeque::len)
    }
}

/// Whether a `kind` from a sender with `status` is held rather than
/// refused: user content from a peer we asked (`PendingOut`), the kinds an
/// `Accepted` friend may open a session with.
pub(crate) fn holds(kind: DmTypeId, status: Status) -> bool {
    status == Some(FriendshipState::PendingOut)
        && matches!(
            kind,
            DmTypeId::DmText | DmTypeId::DmEdit | DmTypeId::DmDelete | DmTypeId::DmReaction
        )
}

/// Hold `opened` (opened as held, [`holds`]). Past the cap its row goes.
/// An accept that committed while it was opening releases it at once.
pub(crate) async fn keep(host: &dyn FriendHost, opened: Opened) {
    let sender = opened.sender_hex.clone();
    let kind = opened.type_id;
    let now = host.clock().now_ms();
    match host.stores().held.lock().hold(opened, now) {
        Ok(()) => {
            tracing::info!(from = %sender, ?kind, "holding a message until our request's accept lands");
        }
        Err(opened) => {
            tracing::info!(from = %sender, ?kind, cap = MAX_HELD_PER_PEER,
                "dropping a message: too many held until the accept lands");
            frame::processed(host, &opened);
            return;
        }
    }
    if state::friendship(host, &sender) == Some(FriendshipState::Accepted) {
        release(host, &sender).await;
    }
}

/// The accept from `peer` committed: apply their held messages in order,
/// each judged by the gate again (a block since drops it); the expired go.
pub(crate) async fn release(host: &dyn FriendHost, peer: &str) {
    let now = host.clock().now_ms();
    let (live, expired) = host.stores().held.lock().take(peer, now);
    for opened in &expired {
        tracing::info!(from = %peer, kind = ?opened.type_id, "dropping an expired held message");
        frame::processed(host, opened);
    }
    if !live.is_empty() {
        tracing::info!(from = %peer, count = live.len(), "applying the messages held for the accept");
    }
    for opened in live {
        if !admission::admitted(host, peer, opened.type_id).await {
            frame::processed(host, &opened);
            continue;
        }
        if opened.stream_seq > 0 {
            crate::operations::dm::stream::on_hint(host, peer);
        }
        if let Routed::Unported(opened) = handlers::handle(host, opened).await {
            tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
                "dropping an unported held message");
        }
    }
}

/// [`release`] on the session scope, for a caller inside an apply.
pub(crate) fn release_soon(host: &dyn FriendHost, peer: &str) {
    let (owned, peer) = (host.owned(), peer.to_string());
    host.scope()
        .spawn_or_drop("apply held messages", async move {
            release(owned.as_ref(), &peer).await;
        });
}

/// How the unlock replay treats a row.
pub(crate) enum Replay {
    Apply,
    Hold,
    Refuse,
}

/// The gate for an `unprocessed` row at the unlock replay: admitted, held
/// again (its sender still `PendingOut`), or refused.
pub(crate) async fn judge_replay(host: &dyn FriendHost, opened: &Opened) -> Replay {
    let Ok(status) = admission::unblocked(host, &opened.sender_hex).await else {
        return Replay::Refuse;
    };
    if holds(opened.type_id, status) {
        return Replay::Hold;
    }
    if admission::frame_admitted(host, &opened.sender_hex, opened.type_id, status, false).await {
        Replay::Apply
    } else {
        Replay::Refuse
    }
}

#[cfg(test)]
#[path = "held_tests.rs"]
mod tests;
