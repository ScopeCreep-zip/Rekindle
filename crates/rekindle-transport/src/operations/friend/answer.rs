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
    .await?
    .ok_or_else(|| FriendError::NotFound(format!("no pending request from {public_key}")))
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
    let reply = answer.reply.clone();
    let list = super::list_record::list_write(host);
    let ended = state::db_call!(host, "store accepted friend", move |conn| {
        rekindle_db::repo::friends::changing_list(conn, &ok, list, |conn| {
            let keys = rekindle_db::repo::friends::RecordKeys {
                profile: profile_key.as_deref(),
                mailbox: mailbox_key.as_deref(),
            };
            let ended = rekindle_db::repo::friends::accept_request(conn, &ok, &pk, &dn, now, keys)?;
            // Their reply inbox, for our first contact (plan RV2).
            if let Some((key, writer)) = &reply {
                rekindle_db::repo::friends::set_peer_reply(conn, &ok, &pk, key, writer)?;
            }
            Ok(ended)
        })
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
    state::echo_failed(host, public_key, ended, "superseded by a new friendship");
    // Their chain, named by their request, adopted now that they are
    // accepted and before our accept can reach them (decision 67 item 2):
    // their first DM lands on a chain we already watch.
    if let Some(chain) = &answer.chain_record_key {
        if !dm::stream::adopt_chain(host, public_key, chain).await {
            return Err(FriendError::Failed(
                "their chain record was not stored".into(),
            ));
        }
    }
    // The friendship is stored: every window shows them now (plan E2.6,
    // GC3: the New Group picker missed a friend whose accept was still
    // opening the session below, a DHT read that can take minutes).
    host.publish(SubscriptionEvent::Friend(FriendEvent::Accepted {
        peer_key: public_key.to_string(),
        display_name: Some(display_name.to_string()),
    }));

    // Our accept opens the session from their record (the profile their
    // request named; plan E2.4, B2). A failure shows on their row with
    // Retry (B7); the friendship stands, and their side stays pending
    // until a session reaches them.
    // Our presence key and our chain record ride the accept itself
    // (decision 67 item 2: `dm::send_friend_accept`), so no FriendsKey row
    // of ours follows it.
    crate::operations::prekeys::open_or_fail(host, public_key, DmTypeId::FriendAccept).await;
    super::game_totals::send(host, public_key).await;
    super::bio::send(host, public_key).await;
    if let Some(profile_key) = &profile_key {
        host.watch_friend(public_key, profile_key).await;
    }
    if let Some(invite_id) = &answer.invite_id {
        invites::mark_accepted(host, invite_id);
    }
    super::list_record::queued(host, "our accept");
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
    let failed = state::db_call!(host, "remove pending request", move |conn| {
        rekindle_db::repo::friends::reject_request(conn, &owner, &pk)
    })
    .await?;
    state::echo_failed(host, public_key, failed, "the request was rejected");
    if let Some(invite_id) = &answer.invite_id {
        invites::mark_rejected(host, invite_id);
    }
    if let (Some(blob), Ok(node)) = (answer.route_blob, host.node()) {
        if !blob.is_empty() {
            dm::route::cache(&node, public_key, blob);
        }
    }
    // Nothing goes to the requester (LV23 review L2, Signal's silent
    // message-request delete): their request expires unanswered at its
    // deadline, as an ignored one does.
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
    // Our `FriendRequest` row goes with the request, in its commit
    // (finding 70), not at the next add.
    let list = super::list_record::list_write(host);
    let failed = state::db_call!(host, "cancel the request", move |conn| {
        rekindle_db::repo::friends::changing_list(conn, &owner, list, |conn| {
            rekindle_db::repo::friends::cancel_request(conn, &owner, &pk)
        })
    })
    .await?;
    state::take(host, public_key);
    state::echo_failed(host, public_key, failed, "the request was cancelled");
    // Their records are released as a removed friend's are, our chain
    // record's writer with them (decision 67 item 2).
    host.unwatch_friend(public_key).await;
    host.sync_reply_inboxes().await;
    super::list_record::queued(host, "a cancel");
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
    let list = super::list_record::list_write(host);
    let (incoming, outgoing) = state::db_call!(host, "expire stale requests", move |conn| {
        let incoming =
            rekindle_db::repo::pending_requests::delete_received_before(conn, &owner, cutoff)?;
        let outgoing = rekindle_db::repo::friends::expire_pending_out(conn, &owner, cutoff, list)?;
        Ok((incoming, outgoing))
    })
    .await?;
    if incoming > 0 {
        tracing::info!(deleted = incoming, "expired stale incoming friend requests");
    }
    for (public_key, failed) in &outgoing {
        state::take(host, public_key);
        state::echo_failed(host, public_key, failed.clone(), "the request expired");
        host.unwatch_friend(public_key).await;
        host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: public_key.clone(),
        }));
    }
    if !outgoing.is_empty() {
        // The expired requests' reply inboxes are released (plan RV2).
        host.sync_reply_inboxes().await;
        super::list_record::queued(host, "an expiry");
        tracing::info!(count = outgoing.len(), "expired stale pending-out friends");
    }
    Ok(())
}
