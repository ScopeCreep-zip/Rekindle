//! A call's `CallInvite` through the outbox (plan CE3, lead decision 30).
//!
//! An invite sent once over a cached route that had died was lost unseen
//! (live CE3: no ack, no retry; the caller rang out silently): Veilid's
//! `app_message` is a statement no one answers (0.5.8
//! `rpc_app_message.rs:24`). So the invite is a `friend_handshake` row due
//! by the invite's own `expires_at` (`Deadline::At`), the pattern of plan
//! CE1's `CallEnd`: it is re-sent with the stream's backoff and brought
//! forward when the peer is reachable again (`route::reachable_again`)
//! until the callee's `CallRinging`, accept or decline settles it
//! ([`settle`]), or it expires. The callee ignores a repeated call id.

use rekindle_codec::capnp_codec::direct::DirectBody;

use super::handshake;
use crate::operations::friend::{FriendError, FriendHost};

/// The outbox row of call `call_id`'s invite, so its answer can settle it.
#[must_use]
pub fn client_id(call_id: &str) -> String {
    format!("call-invite-{call_id}")
}

/// Frame `invite` (a `CallInvite`) and commit it to our handshake stream to
/// `to`, due by `deadline_ms` (ms since the epoch, the invite's
/// `expires_at`). It returns once the row is stored: the outbox sends it.
///
/// # Errors
/// The frame could not be built, or the row could not be stored.
pub async fn commit(
    host: &dyn FriendHost,
    to: &str,
    call_id: &str,
    invite: &DirectBody,
    deadline_ms: i64,
) -> Result<(), FriendError> {
    let payload = handshake::payload(host, to, invite).await?;
    host.outbox()?
        .enqueue_handshake_until(to, client_id(call_id), payload, deadline_ms)
        .await?;
    tracing::debug!(to = %to, call = %call_id, deadline_ms, "call invite committed");
    Ok(())
}

/// Settle call `call_id`'s invite row: `answered` (the callee's ringing,
/// accept or decline arrived) settles it `delivered`; otherwise (we
/// cancelled or lost glare) it is discarded. `false` when no row waits.
///
/// # Errors
/// The storage failed.
pub async fn settle(
    host: &dyn FriendHost,
    call_id: &str,
    answered: bool,
) -> Result<bool, FriendError> {
    let outbox = host.outbox()?;
    let id = client_id(call_id);
    if answered {
        outbox.acknowledge_client(&id).await
    } else {
        outbox.discard(&id).await
    }
}
