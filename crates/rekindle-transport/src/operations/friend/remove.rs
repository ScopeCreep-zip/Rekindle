//! Removing a friend: they are told (`Unfriended`, queued until they ack
//! it), the row stays `Removing` while the notice is queued, and the
//! maintenance task forgets them once nothing is.

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::FriendHost;
use super::{list_record, state};
use crate::operations::dm;

/// Remove `public_key` as a friend.
///
/// # Errors
/// The removal could not be stored.
pub async fn remove(host: &dyn FriendHost, public_key: &str) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    // Always queued: an app message has no delivery guarantee, and the
    // queue entry is what keeps the row until the peer's ack.
    if let Err(e) = dm::build_and_queue(host, public_key, &MessagePayload::Unfriended).await {
        tracing::warn!(to = %public_key, error = %e, "Unfriended notice not queued");
    }
    // The row stays `removing` while the notice is queued (its retries need
    // the friend's routing information, across a restart too).
    let (ok, pk) = (owner, public_key.to_string());
    state::db_call!(host, "mark friend removing", move |conn| {
        rekindle_db::repo::friends::set_state(conn, &ok, &pk, FriendshipState::Removing)?;
        rekindle_db::repo::pending_requests::delete(conn, &ok, &pk)
    })
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
            if let Err(e) = dm::send_to_peer(host, &peer, &MessagePayload::Unfriended).await {
                tracing::warn!(to = %peer, error = %e, "failed to send unfriend notification");
            }
            if let Ok(node) = host.node() {
                dm::route::invalidate(&node, &peer);
            }
            host.unwatch_friend(&peer).await;
            if let Err(e) = list_record::push(host).await {
                tracing::warn!(error = %e, "failed to update DHT friend list after removal");
            }
        });
    Ok(())
}

/// Forget the friends we removed once nothing is queued to them (their
/// `Unfriended` notice was acknowledged or ran out of retries): their rows,
/// their projection entries and their Signal sessions. A removal that spans
/// a restart still ends.
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
        if let Ok(signal) = host.signal() {
            if let Err(e) = signal.delete_session(&public_key) {
                tracing::error!(public_key = %public_key, error = %e,
                    "failed to delete Signal session of removed friend");
            }
        }
        tracing::debug!(public_key = %public_key, "forgot removed friend");
    }
    Ok(())
}
