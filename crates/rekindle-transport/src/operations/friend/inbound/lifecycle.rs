//! The rest of a friendship's life on the receive side: a rejection, being
//! unfriended (and the ack of our own unfriend), a friend's profile moving,
//! and the crossing-request auto-accept.

use rekindle_codec::message::envelope::MessagePayload;
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
    if let Err(e) = dm::send_to_peer(host, sender, &MessagePayload::UnfriendedAck).await {
        tracing::debug!(to = %sender, error = %e, "UnfriendedAck not sent");
    }
    state::take(host, sender);
    if let Ok(signal) = host.signal() {
        if let Err(e) = signal.delete_session(sender) {
            tracing::error!(from = %sender, error = %e,
                "failed to delete Signal session after peer unfriended us");
        }
    }
    host.unwatch_friend(sender).await;
    if let Err(e) = list_record::push(host).await {
        tracing::warn!(error = %e, "failed to update DHT friend list after peer unfriended us");
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: sender.to_string(),
    }));
    tracing::info!(from = %sender, "removed by peer (Unfriended)");
}

/// Our side of a crossing handshake.
enum CrossRole {
    /// We initiate; the session init goes out in our `FriendAccept`.
    Initiator(rekindle_crypto::signal::SessionInitInfo),
    /// The peer initiates; we answer their `FriendAccept`.
    Responder,
    /// The handshake failed and the user was told; both sides are already
    /// `Accepted`, so 'Reset Secure Session' recovers it.
    Failed,
}

/// Drop any prior session with the peer and, when we initiate (the lower
/// identity), establish a new one from their bundle.
fn crossing_session(host: &dyn FriendHost, req: &IncomingFriendRequest<'_>) -> CrossRole {
    let result = (|| -> Result<CrossRole, String> {
        let bundle = rekindle_crypto::signal::PreKeyBundle::from_wire(req.prekey_bundle)
            .map_err(|e| format!("unparseable prekey bundle: {e}"))?;
        let their_identity = hex::decode(req.sender_hex).map_err(|e| format!("sender key: {e}"))?;
        let signal = host.signal().map_err(|e| e.to_string())?;
        signal
            .delete_session(req.sender_hex)
            .map_err(|e| e.to_string())?;
        if !signal
            .initiates_crossing_handshake(&their_identity)
            .map_err(|e| e.to_string())?
        {
            return Ok(CrossRole::Responder);
        }
        let info = signal
            .establish_session(req.sender_hex, &bundle)
            .map_err(|e| e.to_string())?;
        tracing::info!(peer = %req.sender_hex, "established Signal session on cross-request auto-accept");
        Ok(CrossRole::Initiator(info))
    })();
    result.unwrap_or_else(|e| {
        tracing::error!(peer = %req.sender_hex, error = %e, "cross-request Signal handshake failed");
        let peer_label = state::label(host, req.sender_hex);
        host.publish(SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
            title: "Couldn't establish secure session".into(),
            body: format!(
                "Cross-request auto-accept with {peer_label} failed at the Signal \
                 handshake: {e}. Click 'Reset Secure Session' from their friend menu \
                 after verifying their safety number out-of-band."
            ),
        }));
        CrossRole::Failed
    })
}

/// Both sides sent a request: our pending friend becomes `Accepted`, our
/// half of the handshake runs (see [`CrossRole`]), and their profile is
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
    // Exactly one side initiates: the lower identity establishes and sends
    // `FriendAccept` with the session init; the higher side answers it.
    match crossing_session(host, req) {
        CrossRole::Initiator(init) => {
            if let Err(e) = dm::send_friend_accept(host, req.sender_hex, Some(init)).await {
                tracing::warn!(error = %e, "failed to send friend accept for cross-request");
            }
        }
        CrossRole::Responder => {
            tracing::info!(peer = %req.sender_hex, "cross-request: peer initiates — awaiting their FriendAccept");
        }
        CrossRole::Failed => {}
    }
    if !req.profile_dht_key.is_empty() {
        host.watch_friend(req.sender_hex, req.profile_dht_key).await;
    }
    host.publish(SubscriptionEvent::Friend(FriendEvent::Accepted {
        peer_key: req.sender_hex.to_string(),
        display_name: Some(req.display_name.to_string()),
    }));
}
