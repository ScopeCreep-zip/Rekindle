//! When a PreKeyInit names prekeys the responder no longer holds (plan
//! E2.4, D18): the responder's signed `PreKeyRejected` (B5), the
//! initiator's one resend from a fresh slot (B6), and the visible failure
//! after a second rejection (B7). The slot a first contact consumed is
//! refilled on the record after the commit (B8).

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, PreKeyInitBody, PreKeyRejected};
use rekindle_crypto::signal::prekey_record::{base_key_hash, rejection_delay_ms};
use rekindle_db::prekeys::{Publish, RejectionVerdict};
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use crate::operations::dm;
use crate::operations::friend::{session_reset, state, FriendHost};

/// A first contact of ours that opened a session, until the peer answers
/// under it: the kind it carried, and whether it was resent once (B6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstContact {
    pub kind: DmTypeId,
    pub retried: bool,
}

/// Our init carrying `kind` to `peer` went out: remember it, keeping a
/// resend's `retried`.
pub(crate) fn began_first_contact(host: &dyn FriendHost, peer: &str, kind: DmTypeId) {
    if matches!(kind, DmTypeId::FriendAccept | DmTypeId::SessionResetAccept) {
        host.stores()
            .first_contacts
            .lock()
            .entry(peer.to_string())
            .or_insert(FirstContact {
                kind,
                retried: false,
            });
    }
}

/// `peer` answered under the session: the first contact landed.
pub(crate) fn acknowledged(host: &dyn FriendHost, peer: &str) {
    if host.stores().first_contacts.lock().remove(peer).is_some() {
        state::update(host, peer, |f| f.handshake_retrying = false);
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

/// B5: `sender`'s init named prekeys we no longer hold. A sender the gate
/// admits for that init (`PendingOut` for a `FriendAccept`, an outstanding
/// reset of ours for a `SessionResetAccept`) gets a signed rejection after
/// a random 2–10 s delay, at most one per base key and ten per sender per
/// 3 h.
pub(crate) fn reject_init(
    host: &dyn FriendHost,
    sender: [u8; 32],
    sender_hex: &str,
    init: &PreKeyInitBody,
) {
    let admitted = match init.first_type_id {
        DmTypeId::FriendAccept => {
            state::friendship(host, sender_hex) == Some(FriendshipState::PendingOut)
        }
        DmTypeId::SessionResetAccept => session_reset::awaits_accept(host, sender_hex),
        _ => false,
    };
    if !admitted {
        tracing::debug!(from = %sender_hex, kind = ?init.first_type_id,
            "no rejection for this PreKeyInit");
        return;
    }
    let hash = base_key_hash(&init.base_key);
    let rejection = DirectBody::PreKeyRejected(PreKeyRejected {
        signed_prekey_id: init.signed_prekey_id,
        one_time_prekey_id: init.one_time_prekey_id,
        pq_prekey: init.pq_prekey,
        base_key_hash: hash,
    });
    let (owned, peer) = (host.owned(), sender_hex.to_string());
    host.scope().spawn_or_drop("prekey rejection", async move {
        let host = owned.as_ref();
        let (Ok(db), Ok(local)) = (host.db(), host.local_party()) else {
            return;
        };
        let verdict =
            rekindle_db::prekeys::admit_rejection(&db, &local.owner(), &sender, hash, local.now_ms)
                .await;
        match verdict {
            Ok(RejectionVerdict::Send) => {}
            Ok(other) => {
                tracing::debug!(to = %peer, ?other, "PreKeyRejected not sent");
                return;
            }
            Err(e) => {
                tracing::warn!(to = %peer, error = %e, "PreKeyRejected not recorded; not sent");
                return;
            }
        }
        let delay = rejection_delay_ms(host.rng());
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        if let Err(e) = dm::send_to_peer(host, &peer, &rejection).await {
            tracing::info!(to = %peer, error = %e, "PreKeyRejected not delivered");
        }
    });
}

/// B6/B7: `sender` refused the prekeys of our init. Only a rejection that
/// names our unanswered init (it verified under their identity, the frame's
/// signature; its base-key hash matches; nothing decrypted under it)
/// discards that session; the first contact is then resent once from a
/// fresh slot, and a second rejection fails it visibly.
pub(crate) async fn on_rejected(host: &dyn FriendHost, sender: &str, rejected: &PreKeyRejected) {
    let (Ok(db), Ok(local), Ok(peer)) = (
        host.db(),
        host.local_party(),
        dm::frame::peer_address(sender),
    ) else {
        return;
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
            return;
        }
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "PreKeyRejected not applied");
            return;
        }
    }
    let contact = host.stores().first_contacts.lock().get(sender).copied();
    let Some(contact) = contact else {
        // An init that carried a message, not a first contact: the session
        // is gone, and the next send opens a new one from their record.
        tracing::info!(from = %sender, "our session init was refused; the next send re-opens it");
        return;
    };
    if contact.retried {
        failed(
            host,
            sender,
            "they refused our keys twice; try again later".to_string(),
        );
        return;
    }
    if let Some(entry) = host.stores().first_contacts.lock().get_mut(sender) {
        entry.retried = true;
    }
    state::update(host, sender, |f| f.handshake_retrying = true);
    host.publish(SubscriptionEvent::Friend(FriendEvent::HandshakeRetrying {
        peer_key: sender.to_string(),
    }));
    tracing::info!(from = %sender, kind = ?contact.kind, ?rejected.pq_prekey,
        "our first contact was refused; resending from a fresh key");
    if let Err(e) = resend(host, sender, contact.kind).await {
        failed(host, sender, e.to_string());
    }
}

/// The first contact again, from another slot of their record.
async fn resend(
    host: &dyn FriendHost,
    peer: &str,
    kind: DmTypeId,
) -> Result<(), crate::operations::friend::FriendError> {
    let profile = state::friend(host, peer)
        .and_then(|f| f.dht_record_key)
        .ok_or_else(|| {
            crate::operations::friend::FriendError::Conflict(format!(
                "{peer}'s profile is not known"
            ))
        })?;
    let bundle = super::pick_bundle(host, peer, &profile).await?;
    match kind {
        DmTypeId::SessionResetAccept => {
            let frame =
                dm::frame::initiate_frame(host, peer, bundle, &DirectBody::SessionResetAccept)
                    .await?;
            dm::send_frame(host, peer, &frame).await
        }
        _ => dm::send_friend_accept(host, peer, bundle).await,
    }
}

/// B7: the first contact cannot be completed; the user sees it with Retry.
fn failed(host: &dyn FriendHost, peer: &str, reason: String) {
    host.stores().first_contacts.lock().remove(peer);
    state::update(host, peer, |f| {
        f.handshake_retrying = false;
        f.handshake_error = Some(reason.clone());
    });
    host.publish(SubscriptionEvent::Friend(FriendEvent::HandshakeFailed {
        peer_key: peer.to_string(),
        reason,
    }));
}
