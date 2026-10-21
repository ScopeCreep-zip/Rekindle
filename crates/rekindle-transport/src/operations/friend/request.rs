//! The `FriendRequest` we send (plan E2.5, D17), built once for both
//! deliveries: by `app_call` for a friend code, and into an invite's
//! request inbox for an invite.

use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{DirectBody, FriendRequest, RequestAuth};
use rekindle_records::port::RouteClass;

use super::error::FriendError;
use super::host::FriendHost;
use super::state;

/// How long a friend request waits for our route to be published, when it
/// is sent within seconds of unlock (B4/P3.2): a request with no route
/// leaves the receiver no way to answer.
const ROUTE_WAIT: Duration = Duration::from_secs(5);

/// Our request carrying `message`, admitted by `auth`: it names our
/// profile, whose device list leads to our prekey record (plan E2.4), our
/// mailbox, and our route for the answer.
///
/// # Errors
/// Our profile or route is not published yet.
pub(crate) async fn build(
    host: &dyn FriendHost,
    message: &str,
    auth: RequestAuth,
) -> Result<DirectBody, FriendError> {
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
    Ok(DirectBody::FriendRequest(FriendRequest {
        display_name: own.display_name,
        message: message.to_string(),
        profile_dht_key: own.profile_dht_key,
        route_blob,
        mailbox_dht_key: own.mailbox_dht_key,
        auth,
    }))
}
