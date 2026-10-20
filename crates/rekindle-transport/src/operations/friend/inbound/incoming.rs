//! The `FriendRequest` and `FriendAccept` receive handlers: the stored
//! request, the friend state, and the crossing-request short-circuit
//! (`super::lifecycle`). A request's receipt is its `app_call` return; an
//! accept arrives as the first payload of the PreKeyInit that opened our
//! session with the sender (plan E2.3).

use rekindle_codec::capnp_codec::direct::DirectBody;
use rekindle_db::repo::friends::Column;
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::lifecycle::accept_crossing_request;
use super::{IncomingFriendAccept, IncomingFriendRequest};
use crate::operations::dm;
use crate::operations::friend::{invites, state, FriendError, FriendHost};

/// Send `body`; when it cannot go now, queue it for the retry, so the peer
/// is not left re-sending (a lost rejection made the requester retry
/// forever, B10/P3.4). The queue is bounded (20 × 30 s).
async fn send_or_queue(host: &dyn FriendHost, to: &str, body: &DirectBody, what: &str) {
    if let Err(e) = dm::send_to_peer(host, to, body).await {
        tracing::info!(to = %to, error = %e, "{what} send failed, queueing for retry");
        if let Err(e) = dm::build_and_queue(host, to, body).await {
            tracing::warn!(to = %to, error = %e, "failed to queue {what} for retry");
        }
    }
}

/// Store the request; persisted before the event, so the row exists when a
/// window acts on it (B5/P3.1).
async fn persist_request(
    host: &dyn FriendHost,
    req: &IncomingFriendRequest<'_>,
) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let pk = req.sender_hex.to_string();
    let dn = req.display_name.to_string();
    let msg = req.message.to_string();
    let pdk = req.profile_dht_key.to_string();
    let rb = req.route_blob.to_vec();
    let mdk = req.mailbox_dht_key.to_string();
    let iid = req.invite_id.map(str::to_string);
    let now = host.clock().now_ms_i64();
    state::db_call!(host, "store friend request", move |conn| {
        rekindle_db::repo::pending_requests::upsert(
            conn,
            &owner,
            &rekindle_db::repo::pending_requests::Incoming {
                public_key: &pk,
                display_name: &dn,
                message: &msg,
                received_at: now,
                profile_dht_key: &pdk,
                route_blob: &rb,
                mailbox_dht_key: &mdk,
                invite_id: iid.as_deref(),
            },
        )
    })
    .await
}

/// A request that answers one of our invites: a cancelled invite's request
/// is rejected; `false` when the request goes no further.
async fn admit_invite(
    host: &dyn FriendHost,
    req: &IncomingFriendRequest<'_>,
    invite_id: &str,
) -> bool {
    match invites::is_cancelled(host, invite_id).await {
        Ok(false) => {
            invites::mark_responded(host, invite_id, req.sender_hex);
            true
        }
        Ok(true) => {
            tracing::info!(from = %req.sender_hex, %invite_id, "rejecting request for cancelled invite");
            send_or_queue(
                host,
                req.sender_hex,
                &DirectBody::FriendReject,
                "FriendReject",
            )
            .await;
            false
        }
        Err(e) => {
            // Unknown invite state: drop rather than accept a request that
            // may answer a cancelled invite; the sender retries.
            tracing::warn!(from = %req.sender_hex, %invite_id, error = %e,
                "invite status unreadable; dropping request");
            false
        }
    }
}

/// An incoming friend request.
pub(crate) async fn on_friend_request(host: &dyn FriendHost, req: &IncomingFriendRequest<'_>) {
    if !req.route_blob.is_empty() {
        if let Ok(node) = host.node() {
            dm::route::cache(&node, req.sender_hex, req.route_blob.to_vec());
        }
    }

    match state::friendship(host, req.sender_hex) {
        Some(FriendshipState::PendingOut) => {
            tracing::info!(from = %req.sender_hex, "cross-request detected — auto-accepting");
            accept_crossing_request(host, req).await;
            return;
        }
        Some(FriendshipState::Accepted) => {
            // A network retry (our receipt never reached them): the call's
            // return answers it, and nothing else changes.
            tracing::info!(from = %req.sender_hex, "friend request from an accepted friend — a retry");
            return;
        }
        Some(FriendshipState::Removing) => {
            // We removed them: the stale row goes, and the request is a
            // fresh contact.
            if !state::write_forget(host, req.sender_hex).await {
                return;
            }
            state::take(host, req.sender_hex);
            tracing::info!(from = %req.sender_hex,
                "received friend request from Removing peer — treating as new request");
        }
        None => {}
    }

    if let Some(invite_id) = req.invite_id {
        if !admit_invite(host, req, invite_id).await {
            return;
        }
    }
    if let Err(e) = persist_request(host, req).await {
        tracing::warn!(from = %req.sender_hex, error = %e,
            "failed to persist friend request — skipping the event to avoid phantom UI state");
        return;
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::RequestReceived {
        from_key: req.sender_hex.to_string(),
        display_name: req.display_name.to_string(),
        message: req.message.to_string(),
    }));
}

/// An incoming friend accept: the friend `Accepted` with the records it
/// named and their profile watched. The session is already open (the
/// PreKeyInit that carried it).
pub(crate) async fn on_friend_accept(host: &dyn FriendHost, a: &IncomingFriendAccept<'_>) {
    if state::friendship(host, a.sender_hex) == Some(FriendshipState::Removing) {
        tracing::info!(from = %a.sender_hex, "ignoring FriendAccept — friend is being removed");
        return;
    }
    if !a.route_blob.is_empty() {
        if let Ok(node) = host.node() {
            dm::route::cache(&node, a.sender_hex, a.route_blob.clone());
        }
    }
    // The store first, then the projection (D1).
    let stored = match state::owner(host) {
        Ok(owner) => {
            let pk = a.sender_hex.to_string();
            let profile = Some(a.profile_dht_key.to_string()).filter(|k| !k.is_empty());
            let mailbox = Some(a.mailbox_dht_key.to_string()).filter(|k| !k.is_empty());
            state::db_call!(host, "store accepted friend", move |conn| {
                let tx = conn.transaction()?;
                rekindle_db::repo::friends::set_state(&tx, &owner, &pk, FriendshipState::Accepted)?;
                if profile.is_some() {
                    rekindle_db::repo::friends::set(
                        &tx,
                        &owner,
                        &pk,
                        Column::DhtRecordKey,
                        profile,
                    )?;
                }
                if mailbox.is_some() {
                    rekindle_db::repo::friends::set(
                        &tx,
                        &owner,
                        &pk,
                        Column::MailboxDhtKey,
                        mailbox,
                    )?;
                }
                tx.commit()
            })
            .await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = stored {
        tracing::error!(from = %a.sender_hex, error = %e, "accepted friend not stored");
        return;
    }
    state::update(host, a.sender_hex, |f| {
        f.handshake_error = None;
        if !a.profile_dht_key.is_empty() {
            f.dht_record_key = Some(a.profile_dht_key.to_string());
        }
        if !a.mailbox_dht_key.is_empty() {
            f.mailbox_dht_key = Some(a.mailbox_dht_key.to_string());
        }
        f.friendship_state = FriendshipState::Accepted;
    });
    if !a.profile_dht_key.is_empty() {
        host.watch_friend(a.sender_hex, a.profile_dht_key).await;
    }
    let display_name = state::friend(host, a.sender_hex)
        .map_or_else(|| a.sender_hex.to_string(), |f| f.display_name);
    host.publish(SubscriptionEvent::Friend(FriendEvent::Accepted {
        peer_key: a.sender_hex.to_string(),
        display_name: Some(display_name),
    }));
}

/// A friend's accept reached us but its session did not open: the row stays
/// pending and says why (plan C7.25). First contact going wrong, not a
/// broken trusted session, so it never points at the reset flow.
pub(crate) fn on_accept_failed(host: &dyn FriendHost, sender: &str, reason: String) {
    if state::friendship(host, sender) != Some(FriendshipState::PendingOut) {
        return;
    }
    state::update(host, sender, |f| f.handshake_error = Some(reason.clone()));
    host.publish(SubscriptionEvent::Friend(FriendEvent::HandshakeFailed {
        peer_key: sender.to_string(),
        reason,
    }));
}
