//! When a PreKeyInit names prekeys the responder no longer holds (plan
//! E2.4, D18): the responder's signed `PreKeyRejected` (B5), the
//! initiator's one resend from a fresh slot (B6), and the visible failure
//! after a second rejection (B7). The slot a first contact consumed is
//! refilled on the record after the commit (B8).

use rekindle_codec::capnp_codec::direct::{
    DirectBody, DmFrame, DmTypeId, PreKeyInitBody, PreKeyRejected,
};
use rekindle_crypto::signal::prekey_record::Publish;
use rekindle_crypto::signal::prekey_record::{base_key_hash, rejection_delay_ms};
use rekindle_db::prekeys::RejectionVerdict;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use crate::operations::direct::admission::{init_admitted, Status};
use crate::operations::dm;
use crate::operations::friend::{state, FriendHost};

/// A first contact of ours that opened a session, until the peer answers
/// under it: the kind it carried, and whether it was resent once (B6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstContact {
    pub kind: DmTypeId,
    pub retried: bool,
    /// It failed (B7) and waits for the user's Retry.
    pub failed: bool,
}

/// Our init carrying `kind` to `peer` went out: remember it, keeping a
/// resend's `retried` (a failed one is replaced).
pub(crate) fn began_first_contact(host: &dyn FriendHost, peer: &str, kind: DmTypeId) {
    if matches!(kind, DmTypeId::FriendAccept | DmTypeId::SessionResetAccept) {
        let fresh = FirstContact {
            kind,
            retried: false,
            failed: false,
        };
        host.stores()
            .first_contacts
            .lock()
            .entry(peer.to_string())
            .and_modify(|c| {
                if c.failed {
                    *c = fresh;
                }
            })
            .or_insert(fresh);
    }
}

/// `peer` answered under the session: the first contact landed.
pub(crate) fn acknowledged(host: &dyn FriendHost, peer: &str) {
    crate::operations::group_dm::first_contact::acknowledged(host, peer);
    if host.stores().first_contacts.lock().remove(peer).is_some() {
        state::update(host, peer, |f| {
            f.handshake_retrying = false;
            f.handshake_error = None;
        });
    }
}

/// Write the slots a first contact consumed and B8 refilled, after the
/// commit, in the unlock scope.
pub(crate) fn refilled(host: &dyn FriendHost, refills: Vec<Publish>) {
    if refills.is_empty() {
        return;
    }
    let owned = host.owned();
    host.scope()
        .spawn_or_drop("publish refilled prekeys", async move {
            if let Err(e) = super::publish(owned.as_ref(), refills).await {
                tracing::warn!(error = %e, "refilled prekey slot not published");
            }
        });
}

/// B5: `sender`'s init named prekeys we no longer hold. A sender with
/// `status` whom the gate's own predicate admits for that init
/// ([`init_admitted`], audit G9: a friend, or a group member for a group
/// control kind) gets a signed rejection, at most one per base key and ten
/// per sender per 3 h: the framed `PreKeyRejected` to send, or `None`
/// (refused: answered NAK).
///
/// It is framed before its slot is counted (D6): a rejection that could
/// not be built is never counted as sent. A message-borne one is counted
/// once framed, before its delayed send: an `app_message` has no delivery
/// report, so a framed one is as sent as it gets, and counting it first
/// keeps the per-base-key limit over the 2–10 s delay.
///
/// # Errors
/// [`Unjudged`]: our own failure (no identity, the database, framing):
/// nothing was judged, so a call is left unanswered (RV5 R2).
pub(crate) async fn rejection(
    host: &dyn FriendHost,
    sender: [u8; 32],
    sender_hex: &str,
    init: &PreKeyInitBody,
    status: Status,
) -> Result<Option<DmFrame>, Unjudged> {
    let kind = init.first_type_id;
    let hash = base_key_hash(&init.base_key);
    // A reset's init reaches here (a prekey we no longer hold) only past
    // `accept_init`'s check of our stored request (audit D-4).
    if !init_admitted(host, sender_hex, kind, status).await {
        tracing::debug!(from = %sender_hex, ?kind, "no rejection for this PreKeyInit");
        return Ok(None);
    }
    let (Ok(db), Ok(local)) = (host.db(), host.local_party()) else {
        return Err(Unjudged);
    };
    let body = DirectBody::PreKeyRejected(PreKeyRejected {
        signed_prekey_id: init.signed_prekey_id,
        one_time_prekey_id: init.one_time_prekey_id,
        pq_prekey: init.pq_prekey,
        base_key_hash: hash,
    });
    let (frame, _) = dm::framed(host, sender_hex, &body).await.map_err(|e| {
        tracing::info!(to = %sender_hex, error = %e, "PreKeyRejected not framed");
        Unjudged
    })?;
    let verdict =
        rekindle_db::prekeys::admit_rejection(&db, &local.owner(), &sender, hash, local.now_ms)
            .await;
    match verdict {
        Ok(RejectionVerdict::Send) => Ok(Some(frame)),
        Ok(other) => {
            tracing::debug!(to = %sender_hex, ?other, "PreKeyRejected not sent");
            Ok(None)
        }
        Err(e) => {
            tracing::warn!(to = %sender_hex, error = %e, "PreKeyRejected not recorded; not sent");
            Err(Unjudged)
        }
    }
}

/// Our own failure while judging a frame: nothing was judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unjudged;

/// What a `PreKeyRejected` that reached us came to (R4, R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    /// It named our unanswered init and was acted on.
    Applied,
    /// It names no unanswered init of ours (or the reply was no such
    /// rejection): final, a refusal; its nonce stays seen.
    NotOurs,
    /// Our own failure (no identity, the database): nothing was judged.
    Failed,
}

/// B5 for an init that came by `app_call` (finding 46): the rejection,
/// wired as the call's reply, or `None` when it does not wire (ours: the
/// call is left unanswered). It answers a call the sender is already
/// waiting on, whose refusal its timing shows anyway, so it carries no
/// delay.
pub(crate) fn rejection_reply(to: &str, frame: &DmFrame) -> Option<Vec<u8>> {
    dm::frame::wire(frame)
        .map_err(|e| tracing::info!(to = %to, error = %e, "PreKeyRejected reply not wired"))
        .ok()
}

/// B5 for an init that came by message: the framed rejection, sent after a
/// random 2–10 s delay.
pub(crate) fn send_rejection(host: &dyn FriendHost, to: &str, frame: DmFrame) {
    let (owned, peer) = (host.owned(), to.to_string());
    host.scope().spawn_or_drop("prekey rejection", async move {
        let host = owned.as_ref();
        let delay = rejection_delay_ms(host.rng());
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        if let Err(e) = dm::send_frame(host, &peer, &frame).await {
            tracing::info!(to = %peer, error = %e, "PreKeyRejected not delivered");
        }
    });
}

/// B6/B7: `sender` refused the prekeys of our init. Only a rejection that
/// names our unanswered init (it verified under their identity, the frame's
/// signature; its base-key hash matches; nothing decrypted under it)
/// discards that session; the first contact is then resent once from a
/// fresh slot, and a second rejection fails it visibly. What it came to:
/// a first contact's call reply that named no init of ours is a refusal
/// (finding 46 refined); our own failure is no refusal of theirs (R5).
pub(crate) async fn on_rejected(
    host: &dyn FriendHost,
    sender: &str,
    rejected: &PreKeyRejected,
) -> Rejection {
    let (Ok(db), Ok(local), Ok(peer)) = (
        host.db(),
        host.local_party(),
        dm::frame::peer_address(sender),
    ) else {
        return Rejection::Failed;
    };
    let discarded = rekindle_db::ratchet::discard_pending_init(
        &db,
        &local.owner(),
        &peer,
        rejected.base_key_hash,
        local.now_ms,
    )
    .await;
    match discarded {
        Ok(true) => {}
        Ok(false) => {
            tracing::info!(from = %sender, "PreKeyRejected names no unanswered init of ours — ignored");
            return Rejection::NotOurs;
        }
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "PreKeyRejected not applied");
            return Rejection::Failed;
        }
    }
    if crate::operations::group_dm::first_contact::on_rejected(host, sender).await {
        return Rejection::Applied;
    }
    let contact = host.stores().first_contacts.lock().get(sender).copied();
    // A first contact failed by its NAK (finding 46) still has its one
    // resend: the `PreKeyRejected` that followed the NAK sends it.
    let Some(contact) = contact else {
        // No first contact we remember (one lost with a restart, or an init
        // that carried a message): an accepted friend has no session now,
        // so it fails visibly with Retry rather than waiting for a send.
        tracing::info!(from = %sender, "our session init was refused; no first contact pending");
        if state::is_accepted(host, sender) {
            super::failed(
                host,
                sender,
                DmTypeId::FriendAccept,
                "they refused the keys of our secure session".to_string(),
            );
        }
        return Rejection::Applied;
    };
    if contact.retried {
        super::failed(
            host,
            sender,
            contact.kind,
            "they refused our keys twice; try again later".to_string(),
        );
        return Rejection::Applied;
    }
    if let Some(entry) = host.stores().first_contacts.lock().get_mut(sender) {
        entry.retried = true;
        entry.failed = false;
    }
    state::update(host, sender, |f| {
        f.handshake_retrying = true;
        f.handshake_error = None;
    });
    host.publish(SubscriptionEvent::Friend(FriendEvent::HandshakeRetrying {
        peer_key: sender.to_string(),
    }));
    tracing::info!(from = %sender, kind = ?contact.kind, ?rejected.pq_prekey,
        "our first contact was refused; resending from a fresh key");
    if let Err(e) = super::open_first_contact(host, sender, contact.kind).await {
        super::failed(host, sender, contact.kind, e.to_string());
    }
    Rejection::Applied
}
