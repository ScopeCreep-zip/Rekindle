//! The friend-handshake messages (plan E2.3): a request is sealed to the
//! peer's identity and names our profile, whose device list leads to our
//! prekey record (plan E2.4); an accept is the PreKeyInit that opens the
//! session from theirs; a reject is sealed.

use rekindle_codec::capnp_codec::direct::{DirectBody, FriendAccept};
use rekindle_crypto::signal::PreKeyBundle;

use super::handshake;
use crate::operations::friend::{FriendError, FriendHost};

/// Send `request` (built by `friend::request::build`), sealed to `to`:
/// it names our profile, so the receiver can open the session from our
/// prekey record when they accept.
///
/// It is committed to our `friend_handshake` stream to them (plan E2.5)
/// and returns then: the outbox sends it by `app_call`, whose empty return
/// is the receipt (`RequestAcknowledged`, the row `delivered`), and
/// otherwise as any frame, under the same nonce, until it leaves.
///
/// # Errors
/// The request could not be sealed, or the row could not be stored.
pub async fn send_friend_request(
    host: &dyn FriendHost,
    to: &str,
    request: &DirectBody,
) -> Result<(), FriendError> {
    handshake::enqueue(host, to, request).await.map(drop)
}

/// Accept `to`'s request: open the session from `bundle` (picked from their
/// prekey record), carrying our records as its first payload.
///
/// # Errors
/// `NotReady` when our profile and mailbox are not held within 30 s; the
/// bundle is not `to`'s, the session could not be opened, or the send
/// failed.
pub async fn send_friend_accept(
    host: &dyn FriendHost,
    to: &str,
    bundle: PreKeyBundle,
) -> Result<(), FriendError> {
    // The accept names our profile and mailbox: wait for the own-records
    // stage to hold them, as a request and an invite do.
    let own = crate::operations::friend::published_identity(host).await?;
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
    let frame =
        super::frame::initiate_frame(host, to, bundle, &accept, super::frame::Position::default())
            .await?;
    handshake::enqueue_frame(host, to, &frame).await.map(drop)
}
