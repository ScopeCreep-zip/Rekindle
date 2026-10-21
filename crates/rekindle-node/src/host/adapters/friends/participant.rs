//! Our dm participant records (plan E2.5, MSR): one DFLT record per
//! conversation, owned by a random keypair only we hold. Its chain carries
//! our stream to that peer. Created on the first frame to them; the owner
//! keypair is saved to the vault (`VaultKey::DmParticipantOwner`) before
//! the record key is stored in `friends.participant_record_key`, so a
//! stored key always has its writer.

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

/// Our record for `peer`, created when none is stored, or replaced when
/// the stored one has no writer saved for this conversation (the peer's
/// identity rotated: their row moved to the new key, the vault entry did
/// not). A created record's key goes to the peer (`created`).
pub(super) async fn writer(
    host: &HostFriends,
    peer: &str,
) -> Result<ParticipantWriter, FriendError> {
    let owner = session_owner(host)?;
    let db = host.db()?;
    let (o, p) = (owner.clone(), peer.to_string());
    let stored = db
        .call(move |conn| rekindle_db::repo::friends::participant_record_key(conn, &o, &p))
        .await
        .map_err(|e| FriendError::Failed(format!("read participant record: {e}")))?;
    if let (Some(record_key), Some(entry)) = (stored, saved(host, peer)?) {
        if entry.record_key == record_key {
            return Ok(ParticipantWriter {
                record_key,
                writer: entry.keypair,
                created: false,
            });
        }
    }
    let pool = host
        .node()?
        .require_records()
        .map_err(|e| FriendError::NotReady(format!("records: {e}")))?;
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
