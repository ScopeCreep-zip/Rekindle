//! The friend-handshake messages (plan E2.3): a request is sealed to the
//! peer's identity and names our profile, whose device list leads to our
//! prekey record (plan E2.4); an accept is the PreKeyInit that opens the
//! session from theirs; a reject is sealed.

use rekindle_codec::capnp_codec::direct::{DirectBody, FriendAccept, RequestAuth};
use rekindle_crypto::signal::PreKeyBundle;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::send::{call_or_send, send_frame, send_to_peer, Delivery};
use crate::operations::friend::{state, FriendError, FriendHost};

/// Send a friend request, sealed to `to` and naming our profile, so the
/// receiver can open the session from our prekey record when they accept.
/// `auth` admits it: the token of the friend code we were given, or the
/// invite we redeem (plan E2.5, D17).
///
/// It goes by `app_call` when we hold a route to them: the call's empty
/// return is the receipt (the receiver stored it), raising
/// `RequestAcknowledged`. Otherwise — no route, a timeout, a NAK — the same
/// frame, under the same nonce, is sent like any other (refreshed, then
/// queued) and the accept is its receipt.
///
/// # Errors
/// Our profile or route is not published yet, or the send failed.
pub async fn send_friend_request(
    host: &dyn FriendHost,
    to: &str,
    message: &str,
    auth: RequestAuth,
) -> Result<(), FriendError> {
    let request = crate::operations::friend::request::build(host, message, auth).await?;
    if let Delivery::Receipted = call_or_send(host, to, &request).await? {
        host.publish(SubscriptionEvent::Friend(
            FriendEvent::RequestAcknowledged {
                peer_key: to.to_string(),
            },
        ));
    }
    Ok(())
}

/// Accept `to`'s request: open the session from `bundle` (picked from their
/// prekey record), carrying our records as its first payload.
///
/// # Errors
/// The bundle is not `to`'s, the session could not be opened, or the send
/// failed.
pub async fn send_friend_accept(
    host: &dyn FriendHost,
    to: &str,
    bundle: PreKeyBundle,
) -> Result<(), FriendError> {
    let own = state::own_identity(host)?;
    let route_blob = host.node()?.personal_route_blob().unwrap_or_default();
    if route_blob.is_empty() {
        tracing::warn!(
            "sending friend accept with empty route blob — peer will fetch from DHT profile"
        );
    }
    let accept = DirectBody::FriendAccept(FriendAccept {
        profile_dht_key: own.profile_dht_key,
        route_blob,
        mailbox_dht_key: own.mailbox_dht_key,
    });
    let frame = super::frame::initiate_frame(host, to, bundle, &accept).await?;
    send_frame(host, to, &frame).await
}

/// Send a friend rejection.
///
/// # Errors
/// The send failed.
pub async fn send_friend_reject(host: &dyn FriendHost, to: &str) -> Result<(), FriendError> {
    send_to_peer(host, to, &DirectBody::FriendReject).await
}
