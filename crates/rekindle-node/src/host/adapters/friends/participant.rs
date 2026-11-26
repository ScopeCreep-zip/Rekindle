//! Our dm participant records (plan E2.5, MSR): one DFLT record per
//! conversation, owned by a random keypair only we hold. Its chain carries
//! our stream to that peer. Created on the first frame to them; the owner
//! keypair is saved to the vault (`VaultKey::DmParticipantOwner`) before
//! the record key is stored in `friends.participant_record_key`, so a
//! stored key always has its writer. The record is made locally, so the
//! first hints never wait on the network; its empty `HEAD` lands before its
//! key is sent (the `StreamRecord` row waits on
//! `rekindle_records::chain::publish_participant`), so a sent key always
//! names a record a reader finds.

use rekindle_transport::operations::dm::stream::ParticipantWriter;
use rekindle_transport::operations::friend::{session_owner, FriendError, FriendHost};
use rekindle_vault::typed::owner_keys::{self, OwnerEntry};

use super::HostFriends;

fn saved(host: &HostFriends, peer: &str) -> Result<Option<OwnerEntry>, FriendError> {
    let guard = host.ctx().vault.lock();
    let vault = guard.as_ref().ok_or(FriendError::Locked)?;
    owner_keys::load_participant_owner(vault, peer)
        .map_err(|e| FriendError::Failed(format!("participant owner not read: {e}")))
}

/// Our stored record for `peer` with the writer saved for it, when both
/// are held; nothing is created.
pub(super) async fn held(
    host: &HostFriends,
    peer: &str,
) -> Result<Option<ParticipantWriter>, FriendError> {
    let owner = session_owner(host)?;
    let db = host.db()?;
    let p = peer.to_string();
    let stored = db
        .call(move |conn| rekindle_db::repo::friends::participant_record_key(conn, &owner, &p))
        .await
        .map_err(|e| FriendError::Failed(format!("read participant record: {e}")))?;
    Ok(match (stored, saved(host, peer)?) {
        (Some(record_key), Some(entry)) if entry.record_key == record_key => {
            Some(ParticipantWriter {
                record_key,
                writer: entry.keypair,
                created: false,
            })
        }
        _ => None,
    })
}

/// Our record for `peer`, created when none is stored, or replaced when
/// the stored one has no writer saved for this conversation (the peer's
/// identity rotated: their row moved to the new key, the vault entry did
/// not). A created record's key goes to the peer (`created`).
///
/// Single-flight per peer: the check and the creation run under the
/// peer's gate (`FriendStores::participant_gate`), so of two concurrent
/// first frames (or a frame and a rotation's re-key) one creates the
/// record and the other awaits it and uses it; a rotation replaces the
/// record exactly once.
pub(super) async fn writer(
    host: &HostFriends,
    peer: &str,
) -> Result<ParticipantWriter, FriendError> {
    let gate = host.stores().participant_gate(peer);
    let _one = gate.lock().await;
    if let Some(ours) = held(host, peer).await? {
        return Ok(ours);
    }
    let owner = session_owner(host)?;
    let db = host.db()?;
    let pool = host
        .node()?
        .require_records()
        .map_err(|e| FriendError::NotReady(format!("records: {e}")))?;
    // Created locally (Veilid writes nothing at create, 0.5.8
    // `storage_manager/create_record.rs`), so this works with our DHT
    // offline; the empty HEAD lands before the key is sent (module doc).
    let (lease, record_key, keypair) = pool
        .create(&rekindle_records::schema::dm_participant(), None)
        .await
        .map_err(|e| FriendError::NotReady(format!("participant record: {e}")))?;
    pool.release(lease).await;
    {
        let guard = host.ctx().vault.lock();
        let vault = guard.as_ref().ok_or(FriendError::Locked)?;
        let entry = OwnerEntry {
            record_key: record_key.clone(),
            keypair: keypair.clone(),
        };
        owner_keys::persist_participant_owner(vault, peer, &entry)
            .map_err(|e| FriendError::Failed(format!("participant owner not saved: {e}")))?;
    }
    let (p, k) = (peer.to_string(), record_key.clone());
    let kept = db
        .call(move |conn| {
            rekindle_db::repo::friends::set_participant_record_key(conn, &owner, &p, &k)
        })
        .await
        .map_err(|e| FriendError::Failed(format!("store participant record: {e}")))?;
    if !kept {
        return Err(FriendError::NotFound(format!("no friend row for {peer}")));
    }
    tracing::info!(peer = %peer, "dm participant record created");
    Ok(ParticipantWriter {
        record_key,
        writer: keypair,
        created: true,
    })
}

#[cfg(test)]
#[path = "participant_tests.rs"]
mod tests;
