//! Our friend-list record: the Cap'n Proto `FriendEntry` list both hosts
//! read (`rekindle_protocol::dht::friends`), written through the session's
//! record pool, which holds it writable from unlock (plan C7.4).

use rekindle_codec::friend::FriendEntry;
use rekindle_types::friend::FriendshipState;

use super::error::FriendError;
use super::host::FriendHost;
use super::state;

/// Write the friends we keep to our friend-list record. Entries already in
/// it keep their `added_at`; friends being removed are dropped.
///
/// # Errors
/// The record could not be read or written.
pub async fn push(host: &dyn FriendHost) -> Result<(), FriendError> {
    let key = state::own_identity(host)?.friend_list_dht_key;
    if key.is_empty() {
        return Err(FriendError::NotReady("no friend list DHT key".to_string()));
    }
    let records = host.node()?.require_records()?;
    let current = rekindle_protocol::dht::friends::read_friend_list(records.as_ref(), &key)
        .await
        .map_err(|e| FriendError::Failed(format!("read friend list: {e}")))?;
    let now = host.clock().now_ms();
    let entries: Vec<FriendEntry> = state::friends(host)
        .into_iter()
        .filter(|f| f.friendship_state != FriendshipState::Removing)
        .map(|f| {
            let added_at = current
                .friends
                .iter()
                .find(|e| e.public_key == f.public_key)
                .map_or(now, |e| e.added_at);
            FriendEntry {
                public_key: f.public_key,
                nickname: f.nickname,
                group: f.group,
                added_at,
                profile_dht_key: f.dht_record_key,
            }
        })
        .collect();
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
