//! The `FriendRequest` and `FriendAccept` receive handlers: the stored
//! request, the friend state, the `FriendRequestReceived` ack, and the
//! crossing-request short-circuit (`super::lifecycle`).

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_db::repo::friends::Column;
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, NotificationEvent, SubscriptionEvent};

use super::lifecycle::accept_crossing_request;
use super::session::{answer_accept, AcceptOutcome};
use super::{IncomingFriendAccept, IncomingFriendRequest};
use crate::operations::dm;
use crate::operations::friend::{invites, state, FriendError, FriendHost};

/// Send `payload`; when it cannot go now, queue it for the retry, so the
/// peer is not left re-sending (a lost ack or rejection made the requester
/// retry forever, B10/P3.4). The queue is bounded (20 × 30 s).
async fn send_or_queue(host: &dyn FriendHost, to: &str, payload: &MessagePayload, what: &str) {
    if let Err(e) = dm::send_to_peer(host, to, payload).await {
        tracing::info!(to = %to, error = %e, "{what} send failed, queueing for retry");
        if let Err(e) = dm::build_and_queue(host, to, payload).await {
            tracing::warn!(to = %to, error = %e, "failed to queue {what} for retry");
        }
    }
}

/// A request from a peer we already accepted: a network retry (their ack
/// never landed) when the bundle's identity is the one we trust — answered
/// with the ack and nothing else; otherwise a changed identity, which is
/// never trusted silently.
async fn request_from_accepted(host: &dyn FriendHost, req: &IncomingFriendRequest<'_>) {
    let identity_matches = rekindle_crypto::signal::PreKeyBundle::from_wire(req.prekey_bundle)
        .ok()
        .and_then(|bundle| {
            host.signal()
                .ok()?
                .is_trusted_identity(req.sender_hex, &bundle.identity_key)
                .ok()
        })
        .unwrap_or(false);
    if identity_matches {
        tracing::info!(from = %req.sender_hex,
            "FriendRequest from already-Accepted peer with matching identity — treating as retry, re-sending ACK");
        if let Err(e) =
            dm::send_to_peer(host, req.sender_hex, &MessagePayload::FriendRequestReceived).await
        {
            tracing::debug!(to = %req.sender_hex, error = %e,
                "failed to re-send FriendRequestReceived ACK on retry — sender's retry queue will fire again later");
        }
        return;
    }
    // A re-install, or an impersonation: the friendship and session stay
    // untouched until the user verifies the safety number and resets.
    let peer_label = state::label(host, req.sender_hex);
    tracing::error!(from = %req.sender_hex,
        "FriendRequest from already-Accepted peer with DIFFERENT identity_key — leaving existing friendship intact");
    host.publish(SubscriptionEvent::Notification(
        NotificationEvent::SystemAlert {
            title: "Peer's identity key has changed".into(),
            body: format!(
                "{peer_label} sent a friend request with a new identity key. \
             This usually means they re-installed the app, but it could also \
             be an impersonation attempt. Verify their safety number out-of-band \
             (phone call, in person), then click 'Reset Secure Session' from \
             their friend menu to accept the new keys."
            ),
        },
    ));
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
    let pkb = req.prekey_bundle.to_vec();
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
                prekey_bundle: &pkb,
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
                &MessagePayload::FriendReject,
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
    if req.prekey_bundle.is_empty() {
        tracing::warn!(from = %req.sender_hex, "friend request has empty prekey bundle");
    }
    tracing::info!(
        from = %req.sender_hex,
        route_blob_len = req.route_blob.len(),
        route_count = req.route_blob.first().copied().unwrap_or(0),
        "friend request: received peer route blob"
    );
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
            request_from_accepted(host, req).await;
            return;
        }
        Some(FriendshipState::Removing) => {
            // We removed them: the stale row goes, and the request is a
            // fresh contact.
            state::take(host, req.sender_hex);
            state::fire_forget(host, req.sender_hex);
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
            "failed to persist friend request — skipping event and ACK to avoid phantom UI state");
        return;
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::RequestReceived {
        from_key: req.sender_hex.to_string(),
        display_name: req.display_name.to_string(),
        message: req.message.to_string(),
    }));
    send_or_queue(
        host,
        req.sender_hex,
        &MessagePayload::FriendRequestReceived,
        "FriendRequestReceived ACK",
    )
    .await;
}

/// An incoming friend accept: the responder session, then the friend
/// `Accepted` with the records it named and their profile watched. No
/// session, no friendship: the row stays pending and says why (plan C7.25).
pub(crate) async fn on_friend_accept(host: &dyn FriendHost, a: &IncomingFriendAccept<'_>) {
    if state::friendship(host, a.sender_hex) == Some(FriendshipState::Removing) {
        tracing::info!(from = %a.sender_hex, "ignoring FriendAccept — friend is being removed");
        return;
    }
    let outcome = answer_accept(host, a);
    if !a.route_blob.is_empty() {
        if let Ok(node) = host.node() {
            dm::route::cache(&node, a.sender_hex, a.route_blob.clone());
        }
    }
    if let AcceptOutcome::Failed(reason) = outcome {
        state::update(host, a.sender_hex, |f| {
            f.handshake_error = Some(reason.clone());
        });
        host.publish(SubscriptionEvent::Friend(FriendEvent::HandshakeFailed {
            peer_key: a.sender_hex.to_string(),
            reason,
        }));
        return;
    }
    state::fire_state(host, a.sender_hex, FriendshipState::Accepted);
    if !a.profile_dht_key.is_empty() {
        state::fire_set(
            host,
            a.sender_hex,
            Column::DhtRecordKey,
            Some(a.profile_dht_key.to_string()),
        );
    }
    if !a.mailbox_dht_key.is_empty() {
        state::fire_set(
            host,
            a.sender_hex,
            Column::MailboxDhtKey,
            Some(a.mailbox_dht_key.to_string()),
        );
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
