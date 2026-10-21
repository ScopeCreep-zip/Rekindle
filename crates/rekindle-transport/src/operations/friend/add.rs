//! Adding a friend: by public key, or from an invite link. Both leave a
//! `PendingOut` friend and send a friend request; the handshake is the
//! request → accept round trip.

use rekindle_codec::message::envelope::InviteBlob;
use rekindle_types::friend::{FriendState, FriendshipState};
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::FriendHost;
use super::{blocked, invites, state};
use crate::operations::dm;

/// Refuse ourselves and anyone we blocked.
async fn check_addable(
    host: &dyn FriendHost,
    owner: &str,
    public_key: &str,
) -> Result<(), FriendError> {
    if public_key == owner {
        return Err(FriendError::Invalid(
            "You cannot add yourself as a friend".to_string(),
        ));
    }
    if blocked::is_blocked(host, public_key).await? {
        return Err(FriendError::Conflict(
            "Cannot add a blocked user. Unblock them first.".to_string(),
        ));
    }
    Ok(())
}

/// Add a friend by public key: a `PendingOut` friend, and a request sent
/// best-effort (the peer may be offline; the request is queued).
///
/// # Errors
/// The key is malformed, ours, or blocked; our profile is not published
/// yet (`NotReady`: the request must name it); or the friend cannot be
/// stored.
pub async fn add(
    host: &dyn FriendHost,
    public_key: &str,
    message: &str,
) -> Result<(), FriendError> {
    state::require_public_key(public_key)?;
    let owner = state::owner(host)?;
    check_addable(host, &owner, public_key).await?;
    invites::published_identity(host).await?;
    let display_name = state::known_name(host, &owner, public_key).await?;
    let display_name = display_name.as_str();

    let now = host.clock().now_ms_i64();
    let (ok, pk, dn) = (
        owner.clone(),
        public_key.to_string(),
        display_name.to_string(),
    );
    state::db_call!(host, "store friend", move |conn| {
        rekindle_db::repo::friends::insert_pending_out(conn, &ok, &pk, &dn, now)
    })
    .await?;
    state::put(
        host,
        FriendState::new(
            public_key.to_string(),
            display_name.to_string(),
            FriendshipState::PendingOut,
        ),
    );

    if let Err(e) = dm::send_friend_request(host, public_key, message, None).await {
        tracing::warn!(error = %e, "failed to send friend request (peer may be offline)");
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Added {
        peer_key: public_key.to_string(),
        display_name: display_name.to_string(),
        friendship_state: FriendshipState::PendingOut,
        group_label: rekindle_types::dto::friends::group_label(None),
    }));
    host.audit(
        rekindle_audit::AuditKind::FriendAdded,
        serde_json::json!({
            "peer_public_key": public_key,
            "display_name": display_name,
            "direction": "outbound",
        }),
    )
    .await;
    Ok(())
}

/// Add the friend an invite link names: decoded, verified and checked for
/// recency, any stale state from an earlier friendship cleared, then a
/// `PendingOut` friend with the invite's records and a request sent.
///
/// # Errors
/// The invite is malformed, forged or expired; it is ours or a blocked
/// user's; our profile is not published yet (`NotReady`); or the friend
/// cannot be stored.
pub async fn from_invite(host: &dyn FriendHost, link: &str) -> Result<(), FriendError> {
    let blob = invites::open(link, host.clock().now_ms())?;
    let owner = state::owner(host)?;
    check_addable(host, &owner, &blob.public_key).await?;
    invites::published_identity(host).await?;

    if state::take(host, &blob.public_key).is_some() {
        if let Ok(node) = host.node() {
            dm::route::invalidate(&node, &blob.public_key);
        }
    }
    let now = host.clock().now_ms_i64();
    let (ok, pk, dn) = (owner, blob.public_key.clone(), blob.display_name.clone());
    let (profile_key, mailbox_key) = (blob.profile_dht_key.clone(), blob.mailbox_dht_key.clone());
    state::db_call!(host, "store friend", move |conn| {
        rekindle_db::repo::friends::replace_with_invited(
            conn,
            &ok,
            &pk,
            &dn,
            now,
            &profile_key,
            &mailbox_key,
        )
    })
    .await?;
    let mut friend = FriendState::new(
        blob.public_key.clone(),
        blob.display_name.clone(),
        FriendshipState::PendingOut,
    );
    friend.dht_record_key = Some(blob.profile_dht_key.clone());
    friend.mailbox_dht_key = Some(blob.mailbox_dht_key.clone());
    state::put(host, friend);

    setup_invite_contact(host, &blob).await;
    if let Err(e) = dm::send_friend_request(
        host,
        &blob.public_key,
        "Added via invite link",
        blob.invite_id.as_deref(),
    )
    .await
    {
        tracing::warn!(error = %e, "failed to send friend request");
    }
    host.watch_friend(&blob.public_key, &blob.profile_dht_key)
        .await;
    host.publish(SubscriptionEvent::Friend(FriendEvent::Added {
        peer_key: blob.public_key.clone(),
        display_name: blob.display_name.clone(),
        friendship_state: FriendshipState::PendingOut,
        group_label: rekindle_types::dto::friends::group_label(None),
    }));
    tracing::info!(public_key = %blob.public_key, "friend added from invite");
    Ok(())
}

/// Cache the invite's route for immediate contact, end any session left
/// from a prior friendship with this peer, and refresh the
/// route from the peer's mailbox when it has a newer one.
///
/// Deliberately does NOT establish a session: the invite's bundle carries
/// no one-time keys and `FriendRequest` has no session-init fields, so a
/// session started here could never be completed by the peer. The
/// handshake is the `FriendRequest` → `FriendAccept` round trip, as for an
/// add by public key.
async fn setup_invite_contact(host: &dyn FriendHost, blob: &InviteBlob) {
    tracing::info!(
        peer = %blob.public_key,
        route_blob_len = blob.route_blob.len(),
        route_count = blob.route_blob.first().copied().unwrap_or(0),
        "setup_invite_contact: received route blob from invite"
    );
    let Ok(node) = host.node() else {
        return;
    };
    dm::route::cache(&node, &blob.public_key, blob.route_blob.clone());
    state::forget_session(host, &blob.public_key).await;
    let Ok(records) = node.require_records() else {
        return;
    };
    match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(
        records.as_ref(),
        &blob.mailbox_dht_key,
    )
    .await
    {
        Ok(Some(fresh)) if !fresh.is_empty() => {
            dm::route::cache(&node, &blob.public_key, fresh);
            tracing::debug!("refreshed route blob from peer's mailbox");
        }
        _ => tracing::trace!("no fresh route blob in peer mailbox — using invite blob"),
    }
}
