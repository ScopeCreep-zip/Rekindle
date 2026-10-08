//! Phase 23.C — split from friend_runtime.rs. setup_invite_contact body.

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

/// Cache the route blob from an invite for immediate contact, delete any
/// Signal session left from a prior friendship with this peer, and refresh
/// the route blob from the peer's mailbox if available.
///
/// Deliberately does NOT call `establish_session()`: the invite's bundle
/// carries no one-time keys and `FriendRequest` has no session-init
/// fields, so a session started here could never be completed by the
/// peer. The handshake is the `FriendRequest` → `FriendAccept` round trip
/// (`accept_request_inner` initiates; `handle_friend_accept` responds
/// while we are `PendingOut`), identical to add-by-pubkey.
///
/// Called from `add_friend_from_invite`. Failures are logged and do not
/// block invite acceptance.
pub async fn setup_invite_contact(
    state: &Arc<AppState>,
    blob: &rekindle_codec::message::envelope::InviteBlob,
) {
    // Cache the route blob from the invite for immediate contact
    tracing::info!(
        peer = %blob.public_key,
        route_blob_len = blob.route_blob.len(),
        route_count = blob.route_blob.first().copied().unwrap_or(0),
        route_blob_hex_preview = %hex::encode(&blob.route_blob[..blob.route_blob.len().min(32)]),
        "setup_invite_contact: received route blob from invite"
    );
    state_helpers::cache_peer_route(state, &blob.public_key, blob.route_blob.clone());

    // A session from a previous friendship with this peer must not carry
    // messages before the new handshake completes.
    {
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            if let Err(e) = handle.manager.delete_session(&blob.public_key) {
                tracing::error!(peer = %blob.public_key, error = %e,
                    "setup_invite_contact: failed to delete prior Signal session");
            }
        }
    }

    // Try reading the peer's mailbox for a fresh route blob (invite may be stale)
    if let Ok(record_pool) = state_helpers::record_pool(state) {
        match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(
            &record_pool,
            &blob.mailbox_dht_key,
        )
        .await
        {
            Ok(Some(fresh_blob)) if !fresh_blob.is_empty() => {
                state_helpers::cache_peer_route(state, &blob.public_key, fresh_blob);
                tracing::debug!("refreshed route blob from peer's mailbox");
            }
            _ => tracing::trace!("no fresh route blob in peer mailbox — using invite blob"),
        }
    }
}
