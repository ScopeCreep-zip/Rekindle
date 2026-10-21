//! DM read receipts (plan E2.5, r10): opt-in, reciprocal, one global
//! setting, off by default. Our marker `ReadUpTo{stream_seq}` (inside the
//! ratchet, no timestamp) names the newest entry of the peer's stream that
//! our focused window rendered; it goes only to an accepted friend, only
//! while our setting is on, on the delivery receipts' coalescer
//! ([`super::ack`]: the 2–10 s delay, held while Invisible and sent once
//! on leaving it). A received marker raises our entries to `read` in the
//! outbox, and is ignored, never stored, while our own setting is off
//! (Signal `ReceiptMessageProcessor.kt:77-80`, Session
//! `accountManager.ts:185-186`). The setting itself never goes on the
//! wire. Community channels carry no marker.

use rekindle_codec::capnp_codec::direct::ReadUpTo;
use rekindle_types::friend::FriendshipState;

use super::ack;
use crate::operations::friend::{state, FriendError, FriendHost};

/// Our receipts setting (`Preferences.read_receipts_enabled`); off when it
/// cannot be read.
async fn enabled(host: &dyn FriendHost) -> bool {
    let Ok(owner) = state::owner(host) else {
        return false;
    };
    let read = state::db_call!(host, "read the receipts setting", move |conn| {
        rekindle_db::repo::preferences::load(conn, &owner)
    })
    .await;
    match read {
        Ok(prefs) => prefs.read_receipts_enabled,
        Err(e) => {
            tracing::debug!(error = %e, "receipts setting unread: receipts off");
            false
        }
    }
}

/// The policy on both directions: our setting is on and the peer is an
/// accepted friend (never pending, blocked or unknown).
fn allowed(setting_on: bool, friendship: Option<FriendshipState>) -> bool {
    setting_on && friendship == Some(FriendshipState::Accepted)
}

fn friendship(host: &dyn FriendHost, peer: &str) -> Option<FriendshipState> {
    state::friend(host, peer).map(|f| f.friendship_state)
}

/// Whether a marker may go to `peer` now.
pub(crate) async fn may_send(host: &dyn FriendHost, peer: &str) -> bool {
    let friendship = friendship(host, peer);
    allowed(enabled(host).await, friendship)
}

/// Our focused window rendered `peer`'s message `message_id`: hold our
/// marker at its stream seq for the coalescer.
///
/// # Errors
/// The message's seq could not be read.
pub(crate) async fn rendered(
    host: &dyn FriendHost,
    peer: &str,
    message_id: &str,
) -> Result<(), FriendError> {
    if !may_send(host, peer).await {
        return Ok(());
    }
    let owner = state::owner(host)?;
    let (pk, mid) = (peer.to_owned(), message_id.to_owned());
    let seq = state::db_call!(host, "read the rendered message's seq", move |conn| {
        rekindle_db::repo::messages::received_stream_seq(conn, &owner, &pk, &mid)
    })
    .await?;
    if let Some(seq) = seq.and_then(|s| u64::try_from(s).ok()) {
        ack::hold_read(host, peer, seq);
    }
    Ok(())
}

/// `from` read our stream to them through `marker.stream_seq`.
pub(crate) async fn on_read_up_to(host: &dyn FriendHost, from: &str, marker: &ReadUpTo) {
    let friendship = friendship(host, from);
    if !allowed(enabled(host).await, friendship) {
        tracing::trace!(from = %from, "ReadUpTo ignored");
        return;
    }
    let Ok(up_to) = i64::try_from(marker.stream_seq) else {
        return;
    };
    let outbox = match host.outbox() {
        Ok(outbox) => outbox,
        Err(e) => {
            tracing::debug!(error = %e, "ReadUpTo with no outbox");
            return;
        }
    };
    if let Err(e) = outbox.read_up_to(from, up_to).await {
        tracing::warn!(from = %from, error = %e, "ReadUpTo not applied");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_accepted_friend_with_our_setting_on() {
        assert!(allowed(true, Some(FriendshipState::Accepted)));
        assert!(
            !allowed(false, Some(FriendshipState::Accepted)),
            "setting off"
        );
        // A request to us (PendingIn) and a blocked peer hold no friend
        // row: like a stranger, they have no friendship.
        assert!(!allowed(true, None), "non-friend, PendingIn or blocked");
        for state in [FriendshipState::PendingOut, FriendshipState::Removing] {
            assert!(!allowed(true, Some(state)), "{state:?}");
            assert!(!allowed(false, Some(state)), "{state:?}");
        }
    }
}
