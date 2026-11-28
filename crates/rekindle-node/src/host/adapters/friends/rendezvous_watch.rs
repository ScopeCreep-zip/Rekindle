//! The Host's watches on our column of each group's rendezvous record
//! (plan decision 67 item 3, design §3), modelled on the invite inbox
//! watches (`inbox_watch.rs`): a lease per record, watched on exactly our
//! column's subkeys (`dht_layout::group_rendezvous::column`), held from
//! the create or the accept that stored our place to the leave that
//! deleted it, and re-held at unlock for every stored place. A change is
//! read and opened by `operations::group_dm::rendezvous::landed`. The
//! creator's writers of the records it minted are kept here too, and
//! forgotten when it leaves.

use std::sync::Arc;

use rekindle_records::lease::LeaseId;
use rekindle_records::port::{OpenIntent, RecordPort};
use rekindle_transport::operations::friend::{FriendError, FriendHost};
use rekindle_transport::operations::group_dm::rendezvous;
use rekindle_types::dht_layout::group_rendezvous as layout;

use super::HostFriends;

/// Our watched column of one group's rendezvous record.
pub(super) struct HeldColumn {
    group_id: String,
    column: Vec<u32>,
    lease: LeaseId,
}

/// Hold and watch exactly our column of every stored place's record: one
/// not held is opened and watched, one whose place is gone (its group
/// deleted) or moved is released. `fresh` (an unlock) first forgets every
/// held lease: they ended with the last unlock's record pool.
pub(super) async fn sync(host: &HostFriends, fresh: bool) {
    if fresh {
        host.ctx().friend_watches.rendezvous.lock().clear();
    }
    let (Ok(owner), Ok(db)) = (host.ctx().require_owner(), host.ctx().db.current()) else {
        return;
    };
    let rows = db
        .call(move |conn| rekindle_db::repo::group_dm_rendezvous::all(conn, &owner))
        .await;
    let (Ok(rows), Some(pool)) = (rows, host.node().ok().and_then(|node| node.records())) else {
        return;
    };
    let gone: Vec<LeaseId> = {
        let mut held = host.ctx().friend_watches.rendezvous.lock();
        let stale: Vec<String> = held
            .iter()
            .filter(|(key, h)| {
                !rows.iter().any(|r| {
                    &r.record_key == *key
                        && r.group_id == h.group_id
                        && layout::column(r.slot) == h.column
                })
            })
            .map(|(key, _)| key.clone())
            .collect();
        stale
            .iter()
            .filter_map(|key| held.remove(key).map(|h| h.lease))
            .collect()
    };
    for lease in gone {
        pool.release(lease).await;
    }
    for row in rows {
        let held = (host.ctx().friend_watches.rendezvous.lock()).contains_key(&row.record_key);
        if held {
            continue;
        }
        match pool.acquire(&row.record_key, None, OpenIntent::Hold).await {
            Ok(lease) => hold(host, &pool, row.record_key, row.group_id, row.slot, lease).await,
            Err(e) => tracing::warn!(error = %e, "group rendezvous record not opened"),
        }
    }
}

async fn hold(
    host: &HostFriends,
    pool: &Arc<dyn RecordPort>,
    record_key: String,
    group_id: String,
    slot: u8,
    lease: LeaseId,
) {
    let column = layout::column(slot);
    if let Err(e) = pool.watch(lease, &column).await {
        tracing::warn!(error = %e, "group rendezvous column not watched");
    }
    // A watch reports changes after it is held, never what is already
    // there: what landed while we were locked, or before our place was
    // stored, is read once now (plan decision 67 3b; a copy also heard by
    // its call is the ratchet's duplicate).
    let reader = host.clone();
    let (gid, subkeys) = (group_id.clone(), column.clone());
    host.scope()
        .spawn_or_drop("read group rendezvous column", async move {
            for subkey in subkeys {
                if let Some(value) = read(&reader, lease, subkey).await {
                    rendezvous::landed(&reader, &gid, subkey, &value).await;
                }
            }
        });
    let held = HeldColumn {
        group_id,
        column,
        lease,
    };
    let previous = host
        .ctx()
        .friend_watches
        .rendezvous
        .lock()
        .insert(record_key, held);
    if let Some(previous) = previous.filter(|p| p.lease != lease) {
        pool.release(previous.lease).await;
    }
}

impl HostFriends {
    /// A watched record changed: when it is a group's rendezvous record
    /// whose column we watch, open what landed in our column; whether it
    /// was. `value` is the inline value of the first changed subkey.
    pub fn rendezvous_change(
        &self,
        record_key: &str,
        subkeys: &[u32],
        value: Option<&[u8]>,
    ) -> bool {
        let held = {
            let map = self.ctx().friend_watches.rendezvous.lock();
            map.get(record_key)
                .map(|h| (h.group_id.clone(), h.column.clone(), h.lease))
        };
        let Some((group_id, column, lease)) = held else {
            return false;
        };
        let ours: Vec<u32> = subkeys
            .iter()
            .copied()
            .filter(|s| column.contains(s))
            .collect();
        let first = subkeys.first().copied();
        let mut inline = value.map(<[u8]>::to_vec);
        let host = self.clone();
        self.scope()
            .spawn_or_drop("open group rendezvous entry", async move {
                // The carried value first; the network read always runs, so
                // Veilid's report mark clears (`ValueChange`'s contract).
                for subkey in ours {
                    let carried = (first == Some(subkey)).then(|| inline.take()).flatten();
                    if let Some(value) = &carried {
                        rendezvous::landed(&host, &group_id, subkey, value).await;
                    }
                    if let Some(value) = read(&host, lease, subkey).await {
                        if carried.as_ref() != Some(&value) {
                            rendezvous::landed(&host, &group_id, subkey, &value).await;
                        }
                    }
                }
            });
        true
    }
}

async fn read(host: &HostFriends, lease: LeaseId, subkey: u32) -> Option<Vec<u8>> {
    let pool = host.node().ok()?.records()?;
    match pool.get(lease, subkey, true).await {
        Ok(value) => value.map(|v| v.data),
        Err(e) => {
            tracing::debug!(error = %e, "group rendezvous entry not read");
            None
        }
    }
}

fn with_vault<T>(
    host: &HostFriends,
    f: impl FnOnce(&rekindle_vault::VaultStore) -> Result<T, rekindle_vault::VaultError>,
    what: &str,
) -> Result<T, FriendError> {
    let guard = host.ctx().vault.lock();
    let vault = guard
        .as_ref()
        .ok_or_else(|| FriendError::NotReady("the vault is locked".to_string()))?;
    f(vault).map_err(|e| FriendError::Failed(format!("rendezvous writers not {what}: {e}")))
}

/// Keep the writers of `group_id`'s record we minted.
pub(super) fn keep_writers(
    host: &HostFriends,
    group_id: &str,
    mint: &rekindle_dm::RendezvousMint,
) -> Result<(), FriendError> {
    let entry = rekindle_vault::typed::group_rendezvous::RendezvousWriters {
        record_key: mint.record_key.clone(),
        writers: mint.writers.clone(),
    };
    with_vault(
        host,
        |vault| rekindle_vault::typed::group_rendezvous::persist(vault, group_id, &entry),
        "kept",
    )
}

/// The writers of `group_id`'s record, when we minted it.
pub(super) fn writers(
    host: &HostFriends,
    group_id: &str,
) -> Result<Option<rekindle_dm::RendezvousMint>, FriendError> {
    let held = with_vault(
        host,
        |vault| rekindle_vault::typed::group_rendezvous::load(vault, group_id),
        "read",
    )?;
    Ok(held.map(|entry| rekindle_dm::RendezvousMint {
        record_key: entry.record_key,
        writers: entry.writers,
    }))
}

/// Forget the writers of `group_id`'s record (we left it).
pub(super) fn forget_writers(host: &HostFriends, group_id: &str) -> Result<(), FriendError> {
    with_vault(
        host,
        |vault| rekindle_vault::typed::group_rendezvous::forget(vault, group_id),
        "forgotten",
    )
}
