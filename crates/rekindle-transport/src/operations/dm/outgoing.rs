//! The friend-handshake messages (plan E2.3): a request is sealed to the
//! peer's identity and names our profile, whose device list leads to our
//! prekey record (plan E2.4); an accept is the PreKeyInit that opens the
//! session from theirs; a reject is sealed.

use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{DirectBody, FriendAccept, FriendRequest};
use rekindle_crypto::signal::PreKeyBundle;
use rekindle_records::port::RouteClass;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::send::{send_frame, send_to_peer, send_to_peer_call};
use crate::operations::friend::{state, FriendError, FriendHost};

/// How long a friend request waits for our route to be published, when it
/// is sent within seconds of unlock (B4/P3.2): a request with no route
/// leaves the receiver no way to answer.
const ROUTE_WAIT: Duration = Duration::from_secs(5);

/// Send a friend request, sealed to `to` and naming our profile, so the
/// receiver can open the session from our prekey record when they accept.
///
/// It goes by `app_call` when we hold a route to them: the call's return is
/// the receipt, raising `RequestAcknowledged`. Otherwise it is sent like
/// any frame (refreshed, then queued) and the accept is its receipt.
///
/// # Errors
/// Our profile or route is not published yet, or the send failed.
pub async fn send_friend_request(
    host: &dyn FriendHost,
    to: &str,
    message: &str,
    invite_id: Option<&str>,
) -> Result<(), FriendError> {
    let own = state::own_identity(host)?;
    if own.profile_dht_key.is_empty() {
        // A request naming no profile leaves the receiver no prekey
        // record to open the session from (plan E2.4).
        return Err(FriendError::NotReady(
            "our profile is not published yet — try again in a moment".to_string(),
        ));
    }
    let routes = host.node()?.routes();
    let route_blob = tokio::time::timeout(ROUTE_WAIT, routes.available_own(RouteClass::General))
        .await
        .map_err(|_| {
            FriendError::NotReady(
                "Veilid private route not allocated yet — try again in a moment".to_string(),
            )
        })?;
    let request = DirectBody::FriendRequest(FriendRequest {
        display_name: own.display_name,
        message: message.to_string(),
        profile_dht_key: own.profile_dht_key,
        route_blob,
        mailbox_dht_key: own.mailbox_dht_key,
        invite_id: invite_id.map(str::to_string),
    });
    match send_to_peer_call(host, to, &request).await {
        Ok(_) => {
            host.publish(SubscriptionEvent::Friend(
                FriendEvent::RequestAcknowledged {
                    peer_key: to.to_string(),
                },
            ));
            Ok(())
        }
        Err(e) => {
            tracing::debug!(to = %to, error = %e, "friend request not delivered by app_call — sending");
            send_to_peer(host, to, &request).await
        }
    }
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
