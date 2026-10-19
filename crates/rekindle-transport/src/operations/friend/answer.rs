//! Answering friend requests: accepting or rejecting one we received, and
//! cancelling one we sent.

use rekindle_types::friend::{FriendState, FriendshipState};
use rekindle_types::subscription_events::{FriendEvent, NotificationEvent, SubscriptionEvent};

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

/// Tell the user the secure session with `public_key` could not be set up.
fn session_alert(host: &dyn FriendHost, body: String) {
    host.publish(SubscriptionEvent::Notification(
        NotificationEvent::SystemAlert {
            title: "Couldn't establish secure session".into(),
            body,
        },
    ));
}

/// Open the session with `public_key` from the bundle their request
/// carried and send our accept as its first payload. A failure is told to
/// the user; the friendship stands, and their side stays pending until a
/// session reaches them.
async fn open_session(host: &dyn FriendHost, public_key: &str, bundle_bytes: Option<&[u8]>) {
    let peer_label = state::label(host, public_key);
    let Some(bundle_bytes) = bundle_bytes else {
        tracing::warn!(peer = %public_key, "no stored prekey bundle — no session to open");
        session_alert(
            host,
            format!("{peer_label}'s request carried no key bundle. Ask them to send it again."),
        );
        return;
    };
    let bundle = match rekindle_crypto::signal::PreKeyBundle::from_wire(bundle_bytes) {
        Ok(bundle) => bundle,
        Err(e) => {
            tracing::error!(peer = %public_key, error = %e, "stored prekey bundle does not decode");
            session_alert(
                host,
                format!(
                    "The key bundle in {peer_label}'s request is unreadable. Ask them to \
                     send the request again."
                ),
            );
            return;
        }
    };
    if let Err(e) = dm::send_friend_accept(host, public_key, bundle).await {
        tracing::error!(peer = %public_key, error = %e, "friend accept not sent");
        session_alert(
            host,
            format!(
                "The secure session with {peer_label} could not be opened: {e}. Ask them to \
                 send the request again."
            ),
        );
    }
}

/// Accept the pending request from `public_key`: the friend is stored
/// `Accepted` with the records the request named, our accept opens the
/// session, their profile is watched, and the
/// invite it answered is marked accepted.
///
/// # Errors
/// The request could not be read, or the friend not stored.
pub async fn accept(
    host: &dyn FriendHost,
    public_key: &str,
    display_name: &str,
) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let answer = pending_request(host, public_key).await?;
    let now = host.clock().now_ms_i64();
    let (ok, pk, dn) = (
        owner.clone(),
        public_key.to_string(),
        display_name.to_string(),
    );
    let (profile_key, mailbox_key) = (
        answer.profile_dht_key.clone(),
        answer.mailbox_dht_key.clone(),
    );
    state::db_call!(host, "store accepted friend", move |conn| {
        rekindle_db::repo::friends::insert_accepted(conn, &ok, &pk, &dn, now)?;
        rekindle_db::repo::pending_requests::delete(conn, &ok, &pk)?;
        rekindle_db::repo::friends::set_record_keys(
            conn,
            &ok,
            &pk,
            profile_key.as_deref(),
            mailbox_key.as_deref(),
        )
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
    friend.dht_record_key.clone_from(&answer.profile_dht_key);
    friend.mailbox_dht_key.clone_from(&answer.mailbox_dht_key);
    state::put(host, friend);

    open_session(host, public_key, answer.prekey_bundle.as_deref()).await;
    if let Some(profile_key) = &answer.profile_dht_key {
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
        host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: public_key.clone(),
        }));
    }
    if !outgoing.is_empty() {
        tracing::info!(count = outgoing.len(), "expired stale pending-out friends");
    }
    Ok(())
}
