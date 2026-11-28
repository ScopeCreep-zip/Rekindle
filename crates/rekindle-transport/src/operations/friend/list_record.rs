//! Our friend-list record: the Cap'n Proto `FriendEntry` list both hosts
//! read (`rekindle_protocol::dht::friends`), written through the session's
//! record pool, which holds it writable from unlock (plan C7.4), by the
//! outbox's `dht_write` friend-list row (D22).

use rekindle_codec::friend::FriendEntry;
use rekindle_records::port::SetOutcome;
use rekindle_types::friend::FriendshipState;

use super::error::FriendError;
use super::host::FriendHost;
use super::state;

/// Write the friends we keep to our friend-list record, each with the
/// `added_at` its stored row holds; friends being removed are dropped.
/// Built from local state alone, so no network read can lose the write.
/// Called only by the `dht_write` stream's friend-list row ([`queued`]),
/// which owns retry until the outcome is `Landed` (D22).
///
/// # Errors
/// The rows could not be read, there is no record or pool yet, or the
/// record could not be written.
pub async fn write(host: &dyn FriendHost) -> Result<SetOutcome, FriendError> {
    let key = state::own_identity(host)?.friend_list_dht_key;
    if key.is_empty() {
        return Err(FriendError::NotReady("no friend list DHT key".to_string()));
    }
    let records = host.node()?.require_records()?;
    let owner = state::owner(host)?;
    let added: std::collections::HashMap<String, i64> =
        state::db_call!(host, "read when friends were added", move |conn| {
            rekindle_db::repo::friends::added_at(conn, &owner)
        })
        .await?
        .into_iter()
        .collect();
    let now = host.clock().now_ms();
    let mut entries: Vec<FriendEntry> = state::friends(host)
        .into_iter()
        .filter(|f| f.friendship_state != FriendshipState::Removing)
        .map(|f| FriendEntry {
            added_at: added
                .get(&f.public_key)
                .and_then(|at| u64::try_from(*at).ok())
                .unwrap_or(now),
            public_key: f.public_key,
            nickname: f.nickname,
            group: f.group,
            profile_dht_key: f.dht_record_key,
        })
        .collect();
    // The session's friends are a map: a defined order on the wire.
    entries.sort_by(|a, b| a.public_key.cmp(&b.public_key));
    let outcome =
        rekindle_protocol::dht::friends::write_friend_list(records.as_ref(), &key, &entries)
            .await
            .map_err(|e| FriendError::Failed(format!("failed to write the friend list: {e}")))?;
    Ok(outcome)
}

/// How long a friend-list row waits before its first attempt: a burst of
/// friend changes (an accept and its cross request, a removal and its
/// rotations) supersedes one row and writes the record once, and the
/// change's own frames (an `UnfriendedAck`, a `FriendAccept`) leave before
/// the record write competes with them for the node.
const LIST_SETTLE_MS: i64 = 2_000;

/// The friend-list row a change queues in its own transaction
/// (`rekindle_db::repo::friends::changing_list`): now, by the session's
/// clock, due after [`LIST_SETTLE_MS`], with the `DELIVERY_HORIZON`
/// deadline (D24).
pub(crate) fn list_write(host: &dyn FriendHost) -> rekindle_db::repo::outbox::ListWrite {
    let at = host.clock().now_ms_i64();
    let horizon =
        i64::try_from(rekindle_crypto::horizon::DELIVERY_HORIZON.as_millis()).unwrap_or(i64::MAX);
    rekindle_db::repo::outbox::ListWrite {
        at,
        due: at.saturating_add(LIST_SETTLE_MS),
        deadline: at.saturating_add(horizon),
    }
}

/// A change committed our friend list's row with it (review RV5 item 5,
/// live A/B finding 50; `after` names it for the log): wake the outbox,
/// whose `dht_write` stream writes the list from local state off the pump
/// and keeps the row across a lock or crash until it lands. With no
/// outbox running the row waits for the next unlock.
pub(crate) fn queued(host: &dyn FriendHost, after: &'static str) {
    match host.outbox() {
        Ok(outbox) => outbox.wake(),
        Err(e) => tracing::debug!(error = %e, after, "friend list row waits for the outbox"),
    }
}
