//! Adding a friend: by friend code, or from an invite link. Both leave a
//! `PendingOut` friend and send a friend request; the handshake is the
//! request → accept round trip. A request by code carries the code's
//! token; one from an invite names the invite (plan E2.5, D17).

use rekindle_codec::capnp_codec::direct::RequestAuth;
use rekindle_codec::message::envelope::InviteBlob;
use rekindle_types::friend::{FriendState, FriendshipState};
use rekindle_types::friend_code::FriendCode;
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

/// They asked first and their request is held: both of us asked, the
/// crossing seen from our side (`accept_crossing_request` is the same case
/// when ours went first), so we accept theirs, once. A request of ours
/// beside it would wait for an accept they never send: they see a
/// crossing, and as the higher side they await ours (RV2; plan E2.5's
/// handshake is one request answered by one accept).
async fn accept_held(host: &dyn FriendHost, public_key: &str) -> Result<(), FriendError> {
    tracing::info!(peer = %public_key, "adding a peer whose request we hold — accepting it");
    super::answer::accept(host, public_key).await
}

/// Whether a request from `public_key` is held, unanswered.
async fn holds_request_from(
    host: &dyn FriendHost,
    owner: &str,
    public_key: &str,
) -> Result<bool, FriendError> {
    let (owner, peer) = (owner.to_string(), public_key.to_string());
    let held = state::db_call!(host, "read held request", move |conn| {
        rekindle_db::repo::pending_requests::answer(conn, &owner, &peer)
    })
    .await?;
    Ok(held.is_some())
}

/// Add a friend by friend code: a request of theirs we hold is accepted
/// instead ([`holds_request_from`]); otherwise our request built first, then a
/// `PendingOut` friend with the profile the code names, and the request,
/// carrying the code's token and sealed to the code's identity, committed
/// to our `friend_handshake` stream to them (plan E2.5, live finding LF5).
/// Nothing here waits on the network for their route: each attempt of the
/// row resolves it (the route their profile publishes, re-read after a
/// failed call, `dm::send::call_datagram_answer`), the stream's backoff
/// paces it, a new route on their watched profile brings it forward
/// (`route::reachable_again`), and its state reaches the user as the
/// request's send status.
///
/// # Errors
/// The code names us or a blocked user; our profile is not published yet
/// (`NotReady`); or the friend or the request cannot be stored.
pub async fn add(
    host: &dyn FriendHost,
    code: &FriendCode,
    message: &str,
) -> Result<(), FriendError> {
    let public_key = code.public_key.as_str();
    let owner = state::owner(host)?;
    check_addable(host, &owner, public_key).await?;
    if holds_request_from(host, &owner, public_key).await? {
        return accept_held(host, public_key).await;
    }
    invites::published_identity(host).await?;
    // Built before the friend is stored: a request that cannot be built
    // leaves no `PendingOut` friend without a request behind it.
    let auth = RequestAuth::Token(code.token.as_str().to_string());
    let request = super::request::build(host, message, auth).await?;
    let display_name = state::known_name(host, &owner, public_key).await?;
    let display_name = display_name.as_str();

    let now = host.clock().now_ms_i64();
    let (ok, pk, dn, prof) = (
        owner.clone(),
        public_key.to_string(),
        display_name.to_string(),
        code.profile_key.as_str().to_string(),
    );
    // The friend, their record and the end of an ended friendship's
    // handshake rows: one transaction, read against their held request in
    // the same database call (a request stored after the check above).
    let list = super::list_record::list_write(host);
    let stored = state::db_call!(host, "store friend", move |conn| {
        if rekindle_db::repo::pending_requests::answer(conn, &ok, &pk)?.is_some() {
            return Ok(None);
        }
        rekindle_db::repo::friends::changing_list(conn, &ok, list, |conn| {
            rekindle_db::repo::friends::add_by_code(conn, &ok, &pk, &dn, now, &prof)
        })
        .map(Some)
    })
    .await?;
    let Some(ended) = stored else {
        return accept_held(host, public_key).await;
    };
    let mut friend = FriendState::new(
        public_key.to_string(),
        display_name.to_string(),
        FriendshipState::PendingOut,
    );
    friend.dht_record_key = Some(code.profile_key.as_str().to_string());
    state::put(host, friend);
    state::echo_failed(host, public_key, ended, "superseded by a new friendship");
    super::list_record::queued(host, "our request");
    super::request::remember_reply(host, public_key, &request).await?;
    let request = super::request::named_chain(host, public_key, request).await?;

    let committed = dm::send_friend_request(host, public_key, &request).await;
    match &committed {
        Ok(()) => tracing::info!("friend request committed to the outbox"),
        Err(e) => tracing::warn!(error = %e, "friend request not committed"),
    }
    host.watch_friend(public_key, code.profile_key.as_str())
        .await;
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
    // Returned once the request is committed (plan E2.5): its progress
    // arrives as events, and a request that could not be stored fails here.
    committed
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
    // The invite id is what admits the request (an untracked invite admits
    // nothing).
    let invite_id = blob
        .invite_id
        .clone()
        .ok_or_else(|| FriendError::Invalid("the invite names no invite id".to_string()))?;
    let owner = state::owner(host)?;
    check_addable(host, &owner, &blob.public_key).await?;
    if holds_request_from(host, &owner, &blob.public_key).await? {
        return accept_held(host, &blob.public_key).await;
    }
    invites::published_identity(host).await?;

    if state::take(host, &blob.public_key).is_some() {
        if let Ok(node) = host.node() {
            dm::route::invalidate(&node, &blob.public_key);
        }
    }
    let now = host.clock().now_ms_i64();
    let (ok, pk, dn) = (owner, blob.public_key.clone(), blob.display_name.clone());
    let (profile_key, mailbox_key) = (blob.profile_dht_key.clone(), blob.mailbox_dht_key.clone());
    let list = super::list_record::list_write(host);
    let stored = state::db_call!(host, "store invited friend", move |conn| {
        if rekindle_db::repo::pending_requests::answer(conn, &ok, &pk)?.is_some() {
            return Ok(None);
        }
        rekindle_db::repo::friends::changing_list(conn, &ok, list, |conn| {
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
        .map(Some)
    })
    .await?;
    let Some(ended) = stored else {
        return accept_held(host, &blob.public_key).await;
    };
    let mut friend = FriendState::new(
        blob.public_key.clone(),
        blob.display_name.clone(),
        FriendshipState::PendingOut,
    );
    friend.dht_record_key = Some(blob.profile_dht_key.clone());
    friend.mailbox_dht_key = Some(blob.mailbox_dht_key.clone());
    state::put(host, friend);
    state::echo_failed(
        host,
        &blob.public_key,
        ended,
        "superseded by a new friendship",
    );
    super::list_record::queued(host, "an invite");

    setup_invite_contact(host, &blob).await;
    let committed = super::inbox::redeem(host, &blob, invite_id, "Added via invite link").await;
    host.watch_friend(&blob.public_key, &blob.profile_dht_key)
        .await;
    host.publish(SubscriptionEvent::Friend(FriendEvent::Added {
        peer_key: blob.public_key.clone(),
        display_name: blob.display_name.clone(),
        friendship_state: FriendshipState::PendingOut,
        group_label: rekindle_types::dto::friends::group_label(None),
    }));
    tracing::info!(public_key = %blob.public_key, "friend added from invite");
    // Returned once the request is committed (plan E2.5): its progress
    // arrives as events, and a request that could not be stored fails here.
    committed
}

/// End any session left from a prior friendship with this peer, and cache
/// the route in the peer's mailbox for contact once they accept (the
/// invite carries no route: its request goes to the invite's inbox).
///
/// Deliberately does NOT establish a session: the invite's bundle carries
/// no one-time keys and `FriendRequest` has no session-init fields, so a
/// session started here could never be completed by the peer. The
/// handshake is the `FriendRequest` → `FriendAccept` round trip, as for an
/// add by public key.
async fn setup_invite_contact(host: &dyn FriendHost, blob: &InviteBlob) {
    let Ok(node) = host.node() else {
        return;
    };
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
            tracing::debug!("cached the route blob from the peer's mailbox");
        }
        _ => tracing::trace!("no route blob in the peer's mailbox yet"),
    }
}
