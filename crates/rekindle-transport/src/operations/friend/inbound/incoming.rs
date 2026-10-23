//! The `FriendRequest` and `FriendAccept` receive handlers: the stored
//! request, the friend state, and the crossing-request short-circuit
//! (`super::lifecycle`). A request's receipt is its `app_call` return; an
//! accept arrives as the first payload of the PreKeyInit that opened our
//! session with the sender (plan E2.3).

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, RequestAuth};
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::lifecycle::accept_crossing_request;
use super::{IncomingFriendAccept, IncomingFriendRequest};
use crate::operations::dm;
use crate::operations::friend::{invites, state, FriendError, FriendHost};

/// Commit `body` to the outbox, so the peer is not left re-sending (a lost
/// rejection made the requester retry forever, B10/P3.4); the row's
/// deadline bounds it.
async fn enqueue_reply(host: &dyn FriendHost, to: &str, body: &DirectBody, what: &str) {
    if let Err(e) = dm::handshake::enqueue(host, to, body).await {
        tracing::warn!(to = %to, error = %e, "{what} not committed to the outbox");
    }
}

/// Store the request; persisted before the event, so the row exists when a
/// window acts on it (B5/P3.1). Whether it is new: the same request again
/// (a retry whose receipt was lost) changes nothing.
async fn persist_request(
    host: &dyn FriendHost,
    req: &IncomingFriendRequest<'_>,
) -> Result<bool, FriendError> {
    let owner = state::owner(host)?;
    let pk = req.sender_hex.to_string();
    let dn = req.display_name.to_string();
    let msg = req.message.to_string();
    let pdk = req.profile_dht_key.to_string();
    let rb = req.route_blob.to_vec();
    let mdk = req.mailbox_dht_key.to_string();
    let iid = match req.auth {
        RequestAuth::Invite(id) => Some(id.clone()),
        RequestAuth::Token(_) => None,
    };
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

/// A request that answers one of our invites: one answering a cancelled,
/// expired, already-used or unknown invite is rejected; `false` when the
/// request goes no further.
async fn admit_invite(
    host: &dyn FriendHost,
    req: &IncomingFriendRequest<'_>,
    invite_id: &str,
) -> bool {
    match invites::admits_request(host, invite_id, req.sender_hex).await {
        Ok(true) => {
            invites::mark_responded(host, invite_id, req.sender_hex);
            true
        }
        Ok(false) => {
            tracing::info!(from = %req.sender_hex, %invite_id, "rejecting request for a closed invite");
            enqueue_reply(
                host,
                req.sender_hex,
                &DirectBody::FriendReject,
                "FriendReject",
            )
            .await;
            false
        }
        Err(e) => {
            // Unreadable invite state: drop rather than accept a request
            // that may answer a closed invite; the sender retries.
            tracing::warn!(from = %req.sender_hex, %invite_id, error = %e,
                "invite status unreadable; dropping request");
            false
        }
    }
}

/// A request carrying a friend-code token: admitted only when it is our
/// current token, compared in constant time. A stale or wrong token is
/// dropped silently — no reply, so a guesser learns nothing (plan E2.5,
/// D17).
async fn admit_token(host: &dyn FriendHost, req: &IncomingFriendRequest<'_>, token: &str) -> bool {
    let Ok(owner) = state::owner(host) else {
        return false;
    };
    let ours = state::db_call!(host, "read friend token", move |conn| {
        rekindle_db::repo::identity::friend_request_token(conn, &owner)
    })
    .await;
    let admitted = match ours {
        Ok(Some(ours)) => rekindle_types::key_format::friend_request_token(&ours)
            .is_ok_and(|ours| rekindle_secrets::friend_token::matches(&ours, token)),
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(error = %e, "friend token unreadable; dropping request");
            false
        }
    };
    if !admitted {
        tracing::debug!(from = %req.sender_hex, "friend request token refused — dropped");
    }
    admitted
}

/// The admission of a request from a stranger: by our friend-code token,
/// or by one of our invites — the two paths stay apart.
async fn admit(host: &dyn FriendHost, req: &IncomingFriendRequest<'_>) -> bool {
    match req.auth {
        RequestAuth::Token(token) => admit_token(host, req, token).await,
        RequestAuth::Invite(invite_id) => admit_invite(host, req, invite_id).await,
    }
}

/// A request from a friend we accepted. With an established session it is
/// a network retry (our receipt never reached them), and the call's return
/// answers it. With none, or only our unanswered init, our accept never
/// opened a session on their side (it failed, or was lost with a restart):
/// it is opened again (B7), off the call's return.
fn reopen_unanswered(host: &dyn FriendHost, sender: &str) {
    let (owned, peer) = (host.owned(), sender.to_string());
    host.scope().spawn_or_drop("reopen first contact", async move {
        let host = owned.as_ref();
        let (Ok(db), Ok(local), Ok(address)) = (
            host.db(),
            host.local_party(),
            dm::frame::peer_address(&peer),
        ) else {
            return;
        };
        match rekindle_db::ratchet::is_established(&db, &local.owner(), &address).await {
            Ok(true) => {
                tracing::info!(from = %peer, "friend request from an accepted friend — a retry");
            }
            Ok(false) => {
                tracing::info!(from = %peer,
                    "friend request from an accepted friend with no session — opening it again");
                crate::operations::prekeys::open_or_fail(host, &peer, DmTypeId::FriendAccept)
                    .await;
            }
            Err(e) => tracing::warn!(from = %peer, error = %e, "session state unreadable"),
        }
    });
}

/// An incoming friend request, applied once: the same request again stores
/// nothing new and raises no second event (plan audit G4). Whether it is
/// done: `false` when a store failed, so the sender is not receipted.
pub(crate) async fn on_friend_request(
    host: &dyn FriendHost,
    req: &IncomingFriendRequest<'_>,
) -> bool {
    let status = state::friendship(host, req.sender_hex);
    // A stranger (or a friend we are removing) is admitted first; nothing
    // of a refused request is kept, not even its route.
    if matches!(status, None | Some(FriendshipState::Removing)) && !admit(host, req).await {
        return true;
    }
    if !req.route_blob.is_empty() {
        if let Ok(node) = host.node() {
            dm::route::cache(&node, req.sender_hex, req.route_blob.to_vec());
        }
    }

    match status {
        Some(FriendshipState::PendingOut) => {
            tracing::info!(from = %req.sender_hex, "cross-request detected — auto-accepting");
            accept_crossing_request(host, req).await;
            return true;
        }
        Some(FriendshipState::Accepted) => {
            reopen_unanswered(host, req.sender_hex);
            return true;
        }
        Some(FriendshipState::Removing) => {
            // We removed them: the stale row goes, and the request is a
            // fresh contact.
            if !state::write_forget(host, req.sender_hex).await {
                return false;
            }
            state::take(host, req.sender_hex);
            tracing::info!(from = %req.sender_hex,
                "received friend request from Removing peer — treating as new request");
        }
        None => {}
    }

    match persist_request(host, req).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::debug!(from = %req.sender_hex, "friend request already held — a retry");
            return true;
        }
        Err(e) => {
            tracing::warn!(from = %req.sender_hex, error = %e,
                "failed to persist friend request — skipping the event to avoid phantom UI state");
            return false;
        }
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::RequestReceived {
        from_key: req.sender_hex.to_string(),
        display_name: req.display_name.to_string(),
        message: req.message.to_string(),
    }));
    true
}

/// An incoming friend accept: the friend `Accepted` with the records it
/// named and their profile watched. The session is already open (the
/// PreKeyInit that carried it). Whether it is done: `false` when the store
/// failed, so the message is kept for the replay.
pub(crate) async fn on_friend_accept(host: &dyn FriendHost, a: &IncomingFriendAccept<'_>) -> bool {
    if state::friendship(host, a.sender_hex) == Some(FriendshipState::Removing) {
        tracing::info!(from = %a.sender_hex, "ignoring FriendAccept — friend is being removed");
        return true;
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
                let keys = rekindle_db::repo::friends::RecordKeys {
                    profile: profile.as_deref(),
                    mailbox: mailbox.as_deref(),
                };
                rekindle_db::repo::friends::accept_with_records(conn, &owner, &pk, None, keys)
            })
            .await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = stored {
        tracing::error!(from = %a.sender_hex, error = %e, "accepted friend not stored");
        return false;
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
    state::settle_answered_request(host, a.sender_hex).await;
    if !a.profile_dht_key.is_empty() {
        host.watch_friend(a.sender_hex, a.profile_dht_key).await;
    }
    crate::operations::friend::presence_key::send(host, a.sender_hex).await;
    let display_name = state::friend(host, a.sender_hex)
        .map_or_else(|| a.sender_hex.to_string(), |f| f.display_name);
    crate::operations::friend::list_record::push_soon(host, "their accept");
    host.publish(SubscriptionEvent::Friend(FriendEvent::Accepted {
        peer_key: a.sender_hex.to_string(),
        display_name: Some(display_name),
    }));
    true
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
