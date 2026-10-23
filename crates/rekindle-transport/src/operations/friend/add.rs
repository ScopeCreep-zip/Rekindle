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

/// The route the target publishes in the profile its code names (subkey
/// 6), read from the network and cached for the request's `app_call`.
///
/// # Errors
/// `NotReady` when the profile cannot be read or publishes no route.
async fn route_from_code(host: &dyn FriendHost, code: &FriendCode) -> Result<(), FriendError> {
    let node = host.node()?;
    let records = node.require_records()?;
    let blob = rekindle_protocol::dht::profile::read_profile_subkey(
        records.as_ref(),
        code.profile_key.as_str(),
        rekindle_types::dht_layout::profile::ROUTE_BLOB,
        true,
    )
    .await
    .map_err(|e| FriendError::NotReady(format!("their profile could not be read: {e}")))?
    .filter(|blob| !blob.is_empty())
    .ok_or_else(|| FriendError::NotReady("their profile publishes no route yet".to_string()))?;
    dm::route::cache(&node, code.public_key.as_str(), blob);
    Ok(())
}

/// Add a friend by friend code: the target's route read from the profile
/// the code names, a `PendingOut` friend with that profile, and a request
/// carrying the code's token, sealed to the code's identity and sent by
/// `app_call` (an outbox row, sent once the target is reachable).
///
/// # Errors
/// The code names us or a blocked user; our profile is not published yet,
/// or theirs cannot be read or has no route (`NotReady`); or the friend
/// cannot be stored.
pub async fn add(
    host: &dyn FriendHost,
    code: &FriendCode,
    message: &str,
) -> Result<(), FriendError> {
    let public_key = code.public_key.as_str();
    let owner = state::owner(host)?;
    check_addable(host, &owner, public_key).await?;
    invites::published_identity(host).await?;
    route_from_code(host, code).await?;
    let display_name = state::known_name(host, &owner, public_key).await?;
    let display_name = display_name.as_str();

    let now = host.clock().now_ms_i64();
    let (ok, pk, dn, prof) = (
        owner.clone(),
        public_key.to_string(),
        display_name.to_string(),
        code.profile_key.as_str().to_string(),
    );
    state::db_call!(host, "store friend", move |conn| {
        rekindle_db::repo::friends::insert_pending_out(conn, &ok, &pk, &dn, now)?;
        rekindle_db::repo::friends::set_record_keys(conn, &ok, &pk, Some(&prof), None)
    })
    .await?;
    let mut friend = FriendState::new(
        public_key.to_string(),
        display_name.to_string(),
        FriendshipState::PendingOut,
    );
    friend.dht_record_key = Some(code.profile_key.as_str().to_string());
    state::put(host, friend);
    state::settle_ended_handshakes(host, public_key).await?;

    let auth = RequestAuth::Token(code.token.as_str().to_string());
    let committed = dm::send_friend_request(host, public_key, message, auth).await;
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
    state::settle_ended_handshakes(host, &blob.public_key).await?;

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
            tracing::debug!("cached the route blob from the peer's mailbox");
        }
        _ => tracing::trace!("no route blob in the peer's mailbox yet"),
    }
}
