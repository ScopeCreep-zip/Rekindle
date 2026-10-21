//! Removing a friend: they are told (`Unfriended`, an outbox row that waits
//! for their `UnfriendedAck`), the row stays `Removing` while that row is
//! unsettled, and the outbox's settle event forgets them (plan E2.5).

use rekindle_codec::capnp_codec::direct::DirectBody;
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::FriendHost;
use super::{list_record, presence_key, state};
use crate::operations::dm;

/// Remove `public_key` as a friend.
///
/// # Errors
/// The removal could not be stored.
pub async fn remove(host: &dyn FriendHost, public_key: &str) -> Result<(), FriendError> {
    // The state and the `Unfriended` row are one transaction (plan audit
    // D-1): the row keeps the friend's routing information until it
    // settles, across a restart too.
    let payload = dm::handshake::payload(host, public_key, &DirectBody::Unfriended).await?;
    host.outbox()?
        .enqueue_removal(
            public_key,
            dm::handshake::unfriended_client_id(public_key),
            payload,
        )
        .await?;
    host.audit(
        rekindle_audit::AuditKind::FriendRemoved,
        serde_json::json!({ "peer_public_key": public_key }),
    )
    .await;
    state::update(host, public_key, |f| {
        f.friendship_state = FriendshipState::Removing;
    });
    host.publish(SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: public_key.to_string(),
    }));
    tracing::info!(public_key = %public_key, "friend removed");

    let owned = host.owned();
    let peer = public_key.to_string();
    host.scope()
        .spawn_or_drop("friend removal cleanup", async move {
            let host = owned.as_ref();
            host.unwatch_friend(&peer).await;
            if let Err(e) = list_record::push(host).await {
                tracing::warn!(error = %e, "failed to update DHT friend list after removal");
            }
        });
    // Their copy of our presence key opens nothing written from here on.
    presence_key::rotate(host, public_key).await
}

/// Forget the friends we removed once no send to them is unsettled (their
/// `Unfriended` row was acknowledged or failed at its deadline; run from
/// the outbox's settle event): their rows,
/// their projection entries and their sessions, in one transaction. A
/// removal that spans a restart still ends.
///
/// # Errors
/// The removed rows could not be read or deleted.
pub async fn forget_removed(host: &dyn FriendHost) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let forgotten = state::db_call!(host, "forget removed friends", move |conn| {
        rekindle_db::repo::friends::forget_removed(conn, &owner)
    })
    .await?;
    for public_key in forgotten {
        state::take(host, &public_key);
        tracing::debug!(public_key = %public_key, "forgot removed friend");
    }
    Ok(())
}
