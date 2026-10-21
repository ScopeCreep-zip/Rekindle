//! The rest of a friendship's life on the receive side: a rejection, being
//! unfriended (and the ack of our own unfriend), a friend's profile moving,
//! and the crossing-request auto-accept.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};
use rekindle_db::repo::friends::Column;
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, NotificationEvent, SubscriptionEvent};

use super::IncomingFriendRequest;
use crate::operations::dm;
use crate::operations::friend::{list_record, presence_key, state, FriendHost};

/// A friend's profile moved to a new record: watch that one instead.
/// Whether it is done (`false`: the store failed).
pub(crate) async fn on_profile_key_rotated(
    host: &dyn FriendHost,
    sender: &str,
    new_key: &str,
) -> bool {
    if !state::is_accepted(host, sender) {
        return true;
    }
    if !state::write_set(
        host,
        sender,
        Column::DhtRecordKey,
        Some(new_key.to_string()),
    )
    .await
    {
        return false;
    }
    state::update(host, sender, |f| {
        f.dht_record_key = Some(new_key.to_string());
    });
    host.unwatch_friend(sender).await;
    host.watch_friend(sender, new_key).await;
    host.publish(SubscriptionEvent::Friend(FriendEvent::ProfileKeyRotated {
        peer_key: sender.to_string(),
        new_profile_dht_key: new_key.to_string(),
    }));
    tracing::info!(from = %sender, new_key = %new_key, "friend rotated their profile DHT key");
    true
}

/// The peer confirms our `Unfriended`: nothing more is queued to them.
/// Whether it is done (`false`: the queue was not cleared).
pub(crate) async fn on_unfriended_ack(host: &dyn FriendHost, sender: &str) -> bool {
    if !state::acknowledge_unfriended(host, sender).await {
        return false;
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::RemoveAcknowledged {
        peer_key: sender.to_string(),
    }));
    tracing::info!(from = %sender, "received UnfriendedAck — our Unfriended row settled");
    true
}

/// The peer rejected our request: a pending friend of ours goes. Whether
/// it is done (`false`: the pending friend was not forgotten).
pub(crate) async fn on_friend_reject(host: &dyn FriendHost, sender: &str) -> bool {
    let pending = state::friendship(host, sender) == Some(FriendshipState::PendingOut);
    // Named before the row goes: by our nickname or their key fingerprint,
    // never a name from their payload.
    let label = state::label(host, sender);
    let forgotten = pending && state::write_forget(host, sender).await;
    if forgotten {
        state::take(host, sender);
        host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: sender.to_string(),
        }));
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Rejected {
        peer_key: sender.to_string(),
    }));
    if pending {
        host.publish(SubscriptionEvent::Notification(
            NotificationEvent::SystemAlert {
                title: "Friend request declined".to_string(),
                body: format!("{label} declined your friend request"),
            },
        ));
    }
    !pending || forgotten
}

/// The peer removed us: the friend, any request and anything queued to
/// them go with the session (one transaction), they get our ack, their
/// watch stops and our friend-list record follows. Whether it is done
/// (`false`: nothing was deleted).
pub(crate) async fn on_unfriended(host: &dyn FriendHost, sender: &str) -> bool {
    if !matches!(
        state::friendship(host, sender),
        Some(FriendshipState::PendingOut | FriendshipState::Accepted)
    ) {
        tracing::debug!(from = %sender, "ignoring Unfriended from non-friend");
        return true;
    }
    if !state::write_unfriended(host, sender).await {
        return false;
    }
    if let Err(e) = dm::handshake::enqueue(host, sender, &DirectBody::UnfriendedAck).await {
        tracing::warn!(to = %sender, error = %e, "UnfriendedAck not committed to the outbox");
    }
    state::take(host, sender);
    host.unwatch_friend(sender).await;
    if let Err(e) = presence_key::rotate(host, sender).await {
        tracing::warn!(error = %e, "presence key not rotated after peer unfriended us");
    }
    if let Err(e) = list_record::push(host).await {
        tracing::warn!(error = %e, "failed to update DHT friend list after peer unfriended us");
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: sender.to_string(),
    }));
    tracing::info!(from = %sender, "removed by peer (Unfriended)");
    true
}

/// Whether we open the session of a crossing request: exactly one side
/// does, the lower identity, so the two inits do not cross.
fn we_initiate_crossing(host: &dyn FriendHost, peer: &str) -> bool {
    let (Ok(local), Ok(theirs)) = (host.local_party(), hex::decode(peer)) else {
        return false;
    };
    local.owner().as_slice() < theirs.as_slice()
}

/// Both sides sent a request: our pending friend becomes `Accepted`, our
/// half of the handshake runs ([`we_initiate_crossing`]), and their profile is
/// watched.
pub(super) async fn accept_crossing_request(
    host: &dyn FriendHost,
    req: &IncomingFriendRequest<'_>,
) {
    // Stored before anything is sent, so the row that gates the peer's
    // reply is on disk when the reply arrives (plan C7.25), and before the
    // projection (D1).
    let stored = match state::owner(host) {
        Ok(owner) => {
            let pk = req.sender_hex.to_string();
            let name = req.display_name.to_string();
            let profile = Some(req.profile_dht_key.to_string()).filter(|k| !k.is_empty());
            let mailbox = Some(req.mailbox_dht_key.to_string()).filter(|k| !k.is_empty());
            state::db_call!(host, "store accepted state", move |conn| {
                let keys = rekindle_db::repo::friends::RecordKeys {
                    profile: profile.as_deref(),
                    mailbox: mailbox.as_deref(),
                };
                rekindle_db::repo::friends::accept_with_records(
                    conn,
                    &owner,
                    &pk,
                    Some(&name),
                    keys,
                )
            })
            .await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = stored {
        tracing::error!(peer = %req.sender_hex, error = %e,
            "cross-request auto-accept: storing the accepted state failed; not answering");
        return;
    }
    state::update(host, req.sender_hex, |f| {
        f.friendship_state = FriendshipState::Accepted;
        f.display_name = req.display_name.to_string();
        if !req.profile_dht_key.is_empty() {
            f.dht_record_key = Some(req.profile_dht_key.to_string());
        }
        if !req.mailbox_dht_key.is_empty() {
            f.mailbox_dht_key = Some(req.mailbox_dht_key.to_string());
        }
    });
    // Exactly one side initiates: the lower identity opens the session with
    // its accept; the higher side's session opens when that arrives.
    if we_initiate_crossing(host, req.sender_hex) {
        // From the peer's prekey record (the profile their request named,
        // stored above; plan E2.4). A failure shows on their row with Retry
        // (B7); it is a first contact, never a session reset.
        crate::operations::prekeys::open_or_fail(host, req.sender_hex, DmTypeId::FriendAccept)
            .await;
    } else {
        tracing::info!(peer = %req.sender_hex, "cross-request: peer initiates — awaiting their accept");
    }
    if !req.profile_dht_key.is_empty() {
        host.watch_friend(req.sender_hex, req.profile_dht_key).await;
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Accepted {
        peer_key: req.sender_hex.to_string(),
        display_name: Some(req.display_name.to_string()),
    }));
}
