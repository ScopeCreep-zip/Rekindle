//! Our friend-list record: the Cap'n Proto `FriendEntry` list both hosts
//! read (`rekindle_protocol::dht::friends`), written through the session's
//! record pool, which holds it writable from unlock (plan C7.4).

use rekindle_codec::friend::FriendEntry;
use rekindle_types::friend::FriendshipState;

use super::error::FriendError;
use super::host::FriendHost;
use super::state;

/// Write the friends we keep to our friend-list record, each with the
/// `added_at` its stored row holds; friends being removed are dropped.
/// Built from local state alone, so no network read can lose the write:
/// the pool's held write owns the value reaching consensus, and a lock
/// keeps what it still holds as a `dht_write` outbox row (D22 layering).
///
/// # Errors
/// The rows could not be read, or the record could not be written.
pub async fn push(host: &dyn FriendHost) -> Result<(), FriendError> {
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
            .map_err(|e| FriendError::Failed(format!("failed to push friend list update: {e}")))?;
    if outcome.missed() {
        tracing::warn!(?outcome, "friend list update not stored at consensus");
    }
    tracing::debug!(
        count = entries.len(),
        ?outcome,
        "pushed friend list update to DHT"
    );
    Ok(())
}

/// Write our friend-list record off the caller's path, on the session
/// scope (live A/B finding 50: an accept never pushed it, so it never
/// listed the friend). `after` names the change for the log.
pub(crate) fn push_soon(host: &dyn FriendHost, after: &'static str) {
    let owned = host.owned();
    host.scope().spawn_or_drop("friend list push", async move {
        if let Err(e) = push(owned.as_ref()).await {
            tracing::warn!(error = %e, after, "failed to update DHT friend list");
        }
    });
}
