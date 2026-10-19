//! The friend-handshake messages and the typing indicator.

use std::time::Duration;

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_records::port::RouteClass;

use super::send::send_to_peer;
use crate::operations::friend::{state, FriendError, FriendHost};

/// How long a friend request waits for our route to be published, when it
/// is sent within seconds of unlock (B4/P3.2): a request with no route
/// leaves the receiver no way to answer.
const ROUTE_WAIT: Duration = Duration::from_secs(5);

/// A serialized bundle carrying fresh one-time keys, for exactly one peer.
fn handout_bundle_bytes(host: &dyn FriendHost) -> Result<Vec<u8>, FriendError> {
    let bundle = host
        .signal()?
        .handout_bundle()
        .map_err(|e| FriendError::Failed(format!("prekey bundle: {e}")))?;
    Ok(bundle.to_wire())
}

/// Send a friend request, with our handout bundle so the receiver can
/// establish a Signal session. Signed, not encrypted (`Sealing::Plain`):
/// there is no session yet.
///
/// # Errors
/// Our route is not published yet, or the send failed.
pub async fn send_friend_request(
    host: &dyn FriendHost,
    to: &str,
    message: &str,
    invite_id: Option<&str>,
) -> Result<(), FriendError> {
    let own = state::own_identity(host)?;
    let routes = host.node()?.routes();
    let route_blob = tokio::time::timeout(ROUTE_WAIT, routes.available_own(RouteClass::General))
        .await
        .map_err(|_| {
            FriendError::NotReady(
                "Veilid private route not allocated yet — try again in a moment".to_string(),
            )
        })?;
    tracing::info!(
        to = %to,
        route_blob_len = route_blob.len(),
        route_count = route_blob.first().copied().unwrap_or(0),
        "send_friend_request: our route blob info"
    );
    let payload = MessagePayload::FriendRequest {
        display_name: own.display_name,
        message: message.to_string(),
        prekey_bundle: handout_bundle_bytes(host)?,
        profile_dht_key: own.profile_dht_key,
        route_blob,
        mailbox_dht_key: own.mailbox_dht_key,
        invite_id: invite_id.map(str::to_string),
    };
    send_to_peer(host, to, &payload).await
}

/// Send our acceptance, with our handout bundle and, when we established
/// the session, its init so the requester can `respond_to_session`.
///
/// # Errors
/// The send failed.
pub async fn send_friend_accept(
    host: &dyn FriendHost,
    to: &str,
    session_init: Option<rekindle_crypto::signal::SessionInitInfo>,
) -> Result<(), FriendError> {
    let prekey_bundle = handout_bundle_bytes(host)?;
    let own = state::own_identity(host)?;
    let route_blob = host.node()?.personal_route_blob().unwrap_or_default();
    if route_blob.is_empty() {
        tracing::warn!(
            "sending friend accept with empty route blob — peer will fetch from DHT profile"
        );
    }
    let payload = MessagePayload::FriendAccept {
        prekey_bundle,
        profile_dht_key: own.profile_dht_key,
        route_blob,
        mailbox_dht_key: own.mailbox_dht_key,
        ephemeral_key: session_init
            .as_ref()
            .map(|s| s.ephemeral_public_key.clone())
            .unwrap_or_default(),
        signed_prekey_id: session_init.as_ref().map_or(1, |s| s.signed_prekey_id),
        one_time_prekey_id: session_init.as_ref().and_then(|s| s.one_time_prekey_id),
        ml_kem_ciphertext: session_init
            .as_ref()
            .map(|s| s.ml_kem_ciphertext.clone())
            .unwrap_or_default(),
        used_ot_pqpk_id: session_init.as_ref().and_then(|s| s.used_ot_pqpk_id),
    };
    send_to_peer(host, to, &payload).await
}

/// Send a friend rejection.
///
/// # Errors
/// The send failed.
pub async fn send_friend_reject(host: &dyn FriendHost, to: &str) -> Result<(), FriendError> {
    send_to_peer(host, to, &MessagePayload::FriendReject).await
}

/// Send a typing indicator (dropped, never queued, when there is no route).
///
/// # Errors
/// The send failed.
pub async fn send_typing(host: &dyn FriendHost, to: &str, typing: bool) -> Result<(), FriendError> {
    send_to_peer(host, to, &MessagePayload::TypingIndicator { typing }).await
}
