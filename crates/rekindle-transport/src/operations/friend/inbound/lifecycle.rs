//! The rest of a friendship's life on the receive side: a rejection, being
//! unfriended (and the ack of our own unfriend), a friend's profile moving,
//! and the crossing-request auto-accept.

use rekindle_codec::capnp_codec::direct::DirectBody;
use rekindle_db::repo::friends::Column;
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, NotificationEvent, SubscriptionEvent};

use super::IncomingFriendRequest;
use crate::operations::dm;
use crate::operations::friend::{list_record, state, FriendHost};

/// A friend's profile moved to a new record: watch that one instead.
pub(crate) async fn on_profile_key_rotated(host: &dyn FriendHost, sender: &str, new_key: &str) {
    if !state::is_accepted(host, sender) {
        return;
    }
    host.unwatch_friend(sender).await;
    state::update(host, sender, |f| {
        f.dht_record_key = Some(new_key.to_string());
    });
    state::fire_set(
        host,
        sender,
        Column::DhtRecordKey,
        Some(new_key.to_string()),
    );
    host.watch_friend(sender, new_key).await;
    host.publish(SubscriptionEvent::Friend(FriendEvent::ProfileKeyRotated {
        peer_key: sender.to_string(),
        new_profile_dht_key: new_key.to_string(),
    }));
    tracing::info!(from = %sender, new_key = %new_key, "friend rotated their profile DHT key");
}

/// The peer confirms our `Unfriended`: nothing more is queued to them.
pub(crate) fn on_unfriended_ack(host: &dyn FriendHost, sender: &str) {
    state::fire_drop_queued(host, sender);
    host.publish(SubscriptionEvent::Friend(FriendEvent::RemoveAcknowledged {
        peer_key: sender.to_string(),
    }));
    tracing::info!(from = %sender, "received UnfriendedAck — cleared pending messages");
}

/// The peer rejected our request: a pending friend of ours goes.
pub(crate) fn on_friend_reject(host: &dyn FriendHost, sender: &str) {
    if state::friendship(host, sender) == Some(FriendshipState::PendingOut) {
        state::fire_forget(host, sender);
        state::take(host, sender);
        host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: sender.to_string(),
        }));
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Rejected {
        peer_key: sender.to_string(),
    }));
}

/// The peer removed us: the friend, any request and anything queued to
/// them go, they get our ack, the session ends with the friendship, their
/// watch stops and our friend-list record follows.
pub(crate) async fn on_unfriended(host: &dyn FriendHost, sender: &str) {
    if !matches!(
        state::friendship(host, sender),
        Some(FriendshipState::PendingOut | FriendshipState::Accepted)
    ) {
        tracing::debug!(from = %sender, "ignoring Unfriended from non-friend");
        return;
    }
    state::fire_forget(host, sender);
    state::fire_drop_queued(host, sender);
    if let Err(e) = dm::send_to_peer(host, sender, &DirectBody::UnfriendedAck).await {
        tracing::debug!(to = %sender, error = %e, "UnfriendedAck not sent");
    }
    state::take(host, sender);
    state::forget_session(host, sender).await;
    host.unwatch_friend(sender).await;
    if let Err(e) = list_record::push(host).await {
        tracing::warn!(error = %e, "failed to update DHT friend list after peer unfriended us");
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: sender.to_string(),
    }));
    tracing::info!(from = %sender, "removed by peer (Unfriended)");
}

/// Whether we open the session of a crossing request: exactly one side
/// does, the lower identity, so the two inits do not cross.
fn we_initiate_crossing(host: &dyn FriendHost, peer: &str) -> bool {
    let (Ok(local), Ok(theirs)) = (host.local_party(), hex::decode(peer)) else {
        return false;
    };
    local.owner().as_slice() < theirs.as_slice()
}

/// Open the session of a crossing request from the peer's bundle, our
/// accept as its first payload. A failure is told to the user; both sides
/// are already `Accepted`, so a session reset recovers it.
async fn open_crossing_session(host: &dyn FriendHost, req: &IncomingFriendRequest<'_>) {
    let sent = match rekindle_crypto::signal::PreKeyBundle::from_wire(req.prekey_bundle) {
        Ok(bundle) => dm::send_friend_accept(host, req.sender_hex, bundle).await,
        Err(e) => Err(crate::operations::friend::FriendError::Invalid(format!(
            "unreadable prekey bundle: {e}"
        ))),
    };
    if let Err(e) = sent {
        tracing::error!(peer = %req.sender_hex, error = %e, "cross-request session not opened");
        let peer_label = state::label(host, req.sender_hex);
        host.publish(SubscriptionEvent::Notification(
            NotificationEvent::SystemAlert {
                title: "Couldn't establish secure session".into(),
                body: format!(
                    "Cross-request auto-accept with {peer_label} could not open the secure \
                 session: {e}. Reset the secure session from their friend menu after \
                 verifying their safety number out-of-band."
                ),
            },
        ));
    }
}

/// Both sides sent a request: our pending friend becomes `Accepted`, our
/// half of the handshake runs ([`we_initiate_crossing`]), and their profile is
/// watched.
pub(super) async fn accept_crossing_request(
    host: &dyn FriendHost,
    req: &IncomingFriendRequest<'_>,
) {
    state::fire_drop_request(host, req.sender_hex);
    // Stored before anything is sent, so the row that gates the peer's
    // reply is on disk when the reply arrives (plan C7.25).
    let stored = match state::owner(host) {
        Ok(owner) => {
            let pk = req.sender_hex.to_string();
            state::db_call!(host, "store accepted state", move |conn| {
                rekindle_db::repo::friends::set_state(conn, &owner, &pk, FriendshipState::Accepted)
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
    state::fire_set(
        host,
        req.sender_hex,
        Column::DisplayName,
        Some(req.display_name.to_string()),
    );
    if !req.profile_dht_key.is_empty() {
        state::fire_set(
            host,
            req.sender_hex,
            Column::DhtRecordKey,
            Some(req.profile_dht_key.to_string()),
        );
    }
    if !req.mailbox_dht_key.is_empty() {
        state::fire_set(
            host,
            req.sender_hex,
            Column::MailboxDhtKey,
            Some(req.mailbox_dht_key.to_string()),
        );
    }
    // Exactly one side initiates: the lower identity opens the session with
    // its accept; the higher side's session opens when that arrives.
    if we_initiate_crossing(host, req.sender_hex) {
        open_crossing_session(host, req).await;
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
