//! Answering friend requests: accepting or rejecting one we received, and
//! cancelling one we sent.

use rekindle_codec::capnp_codec::direct::DmTypeId;
use rekindle_types::friend::{FriendState, FriendshipState};
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::FriendHost;
use super::{invites, state};
use crate::operations::dm;

/// What answering the pending request from `public_key` needs.
async fn pending_request(
    host: &dyn FriendHost,
    public_key: &str,
) -> Result<rekindle_db::repo::pending_requests::Answer, FriendError> {
    let owner = state::owner(host)?;
    let pk = public_key.to_string();
    state::db_call!(host, "read pending request", move |conn| {
        rekindle_db::repo::pending_requests::answer(conn, &owner, &pk)
    })
    .await
}

/// Accept the pending request from `public_key`: the friend is stored
/// `Accepted` with the records the request named, our accept opens the
/// session, their profile is watched, and the
/// invite it answered is marked accepted.
///
/// # Errors
/// The request could not be read, or the friend not stored.
pub async fn accept(host: &dyn FriendHost, public_key: &str) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let answer = pending_request(host, public_key).await?;
    // The name their request carried (plan audit U-7).
    let display_name = answer
        .display_name
        .clone()
        .unwrap_or_else(|| state::label(host, public_key));
    let display_name = display_name.as_str();
    let now = host.clock().now_ms_i64();
    let (ok, pk, dn) = (
        owner.clone(),
        public_key.to_string(),
        display_name.to_string(),
    );
    // A request naming an empty record names none (a modified client's).
    let named = |key: &Option<String>| key.clone().filter(|k| !k.is_empty());
    let (profile_key, mailbox_key) = (
        named(&answer.profile_dht_key),
        named(&answer.mailbox_dht_key),
    );
    state::db_call!(host, "store accepted friend", move |conn| {
        let keys = rekindle_db::repo::friends::RecordKeys {
            profile: profile_key.as_deref(),
            mailbox: mailbox_key.as_deref(),
        };
        rekindle_db::repo::friends::accept_request(conn, &ok, &pk, &dn, now, keys)
    })
    .await?;

    if let (Some(blob), Ok(node)) = (answer.route_blob.as_ref(), host.node()) {
        if !blob.is_empty() {
            dm::route::cache(&node, public_key, blob.clone());
        }
    }
    let mut friend = FriendState::new(
        public_key.to_string(),
        display_name.to_string(),
        FriendshipState::Accepted,
    );
    friend.dht_record_key = named(&answer.profile_dht_key);
    friend.mailbox_dht_key = named(&answer.mailbox_dht_key);
    let profile_key = friend.dht_record_key.clone();
    state::put(host, friend);
    state::settle_ended_handshakes(host, public_key).await?;

    // Our accept opens the session from their record (the profile their
    // request named; plan E2.4, B2). A failure shows on their row with
    // Retry (B7); the friendship stands, and their side stays pending
    // until a session reaches them.
    crate::operations::prekeys::open_or_fail(host, public_key, DmTypeId::FriendAccept).await;
    super::presence_key::send(host, public_key).await;
    if let Some(profile_key) = &profile_key {
        host.watch_friend(public_key, profile_key).await;
    }
    if let Some(invite_id) = &answer.invite_id {
        invites::mark_accepted(host, invite_id);
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Accepted {
        peer_key: public_key.to_string(),
        display_name: Some(display_name.to_string()),
    }));
    host.audit(
        rekindle_audit::AuditKind::FriendAdded,
        serde_json::json!({
            "peer_public_key": public_key,
            "display_name": display_name,
            "direction": "inbound",
        }),
    )
    .await;
    Ok(())
}

/// Reject the pending request from `public_key` and tell them.
///
/// # Errors
/// The request could not be read or removed.
pub async fn reject(host: &dyn FriendHost, public_key: &str) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let answer = pending_request(host, public_key).await?;
    let pk = public_key.to_string();
    state::db_call!(host, "remove pending request", move |conn| {
        rekindle_db::repo::pending_requests::delete(conn, &owner, &pk)
    })
    .await?;
    state::discard_pending_dms(host, public_key, "the request was rejected").await?;
    if let Some(invite_id) = &answer.invite_id {
        invites::mark_rejected(host, invite_id);
    }
    if let (Some(blob), Ok(node)) = (answer.route_blob, host.node()) {
        if !blob.is_empty() {
            dm::route::cache(&node, public_key, blob);
        }
    }
    if let Err(e) = dm::send_friend_reject(host, public_key).await {
        tracing::warn!(error = %e, "failed to send friend reject (peer may be offline)");
    }
    tracing::info!(public_key = %public_key, "friend request rejected");
    Ok(())
}

/// Cancel the request we sent `public_key`.
///
/// # Errors
/// `Conflict` when no request of ours to them is pending.
pub async fn cancel(host: &dyn FriendHost, public_key: &str) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    if state::friendship(host, public_key) != Some(FriendshipState::PendingOut) {
        return Err(FriendError::Conflict(
            "Not a pending outbound request".to_string(),
        ));
    }
    let pk = public_key.to_string();
    state::db_call!(host, "remove friend", move |conn| {
        rekindle_db::repo::friends::delete(conn, &owner, &pk)
    })
    .await?;
    state::take(host, public_key);
    state::discard_pending_dms(host, public_key, "the request was cancelled").await?;
    host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: public_key.to_string(),
    }));
    tracing::info!(public_key = %public_key, "pending friend request cancelled");
    Ok(())
}

/// How long an unanswered request lives, either way.
const REQUEST_LIFETIME_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Expire requests older than 30 days: incoming ones we never answered,
/// and our pending friends who never answered ours (the maintenance task).
///
/// # Errors
/// The expiry could not be written.
pub async fn expire_stale_requests(host: &dyn FriendHost) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let cutoff = host.clock().now_ms_i64() - REQUEST_LIFETIME_MS;
    let (incoming, outgoing) = state::db_call!(host, "expire stale requests", move |conn| {
        let incoming =
            rekindle_db::repo::pending_requests::delete_received_before(conn, &owner, cutoff)?;
        let outgoing = rekindle_db::repo::friends::delete_pending_out_before(conn, &owner, cutoff)?;
        Ok((incoming, outgoing))
    })
    .await?;
    if incoming > 0 {
        tracing::info!(deleted = incoming, "expired stale incoming friend requests");
    }
    for public_key in &outgoing {
        state::take(host, public_key);
        state::discard_pending_dms(host, public_key, "the request expired").await?;
        host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: public_key.clone(),
        }));
    }
    if !outgoing.is_empty() {
        tracing::info!(count = outgoing.len(), "expired stale pending-out friends");
    }
    Ok(())
}
