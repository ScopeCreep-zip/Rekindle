//! `rekindle friend accept/reject` — handle pending friend requests.

use anyhow::Context;

use rekindle_transport::operations::friend;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Accept a pending friend request.
///
/// Looks up the request by the requester's public key (the request_id),
/// extracts the stored route blob, profile key, and prekey bundle,
/// then calls the transport's `accept_friend_request` operation which:
/// 1. Establishes a Signal session from the prekey bundle
/// 2. Sends a `FriendAccept` DM with our prekey + route
/// 3. Adds the requester to our friend list DHT record
/// 4. Caches their route for future messaging
pub async fn cmd_accept(
    handle: &TransportHandle,
    session: &Session,
    request_id: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        anyhow::bail!("request ID is required (the requester's public key)");
    }

    let pending = session
        .pending_request_by_key(request_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no pending friend request from '{}'.\n\
                 list pending requests: rekindle friend requests",
                helpers::abbreviate_key(request_id)
            )
        })?;

    let signing_key = crate::identity::keystore::load_signing_key()
        .await
        .map_err(|e| crate::error::CliError::Auth(format!("cannot load signing key: {e}")))?;

    friend::accept_friend_request(
        handle.node(),
        session,
        &pending.public_key,
        &pending.route_blob,
        &pending.profile_dht_key,
        &signing_key,
    )
    .await
    .context("failed to accept friend request")?;

    // Remove from pending and save session
    let mut updated_session = session.clone();
    updated_session.remove_pending_friend_request(&pending.public_key);
    let session_path = helpers::session_path()?;
    updated_session.save(&session_path)?;

    let display_name = helpers::sanitize_for_display(&pending.display_name);

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "accepted",
                "public_key": pending.public_key,
                "display_name": display_name,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Friend request from {display_name} ({}) accepted.",
            helpers::abbreviate_key(&pending.public_key)
        ))
    }
}

/// Reject a pending friend request.
///
/// Looks up the request, sends a `FriendReject` DM so the requester
/// knows the request was explicitly declined (not just ignored), then
/// removes the request from the session.
pub async fn cmd_reject(
    handle: &TransportHandle,
    session: &Session,
    request_id: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        anyhow::bail!("request ID is required (the requester's public key)");
    }

    let pending = session
        .pending_request_by_key(request_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no pending friend request from '{}'.\n\
                 list pending requests: rekindle friend requests",
                helpers::abbreviate_key(request_id)
            )
        })?;

    let signing_key = crate::identity::keystore::load_signing_key().await?;

    friend::reject_friend_request(
        handle.node(),
        session,
        &pending.public_key,
        &pending.route_blob,
        &signing_key,
    )
    .await
    .context("failed to send rejection")?;

    // Remove from pending and save session
    let mut updated_session = session.clone();
    updated_session.remove_pending_friend_request(&pending.public_key);
    let session_path = helpers::session_path()?;
    updated_session.save(&session_path)?;

    let display_name = helpers::sanitize_for_display(&pending.display_name);

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "rejected",
                "public_key": pending.public_key,
                "display_name": display_name,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Friend request from {display_name} rejected.",
        ))
    }
}
