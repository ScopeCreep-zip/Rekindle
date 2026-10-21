//! Our group chain per group (plan E2.6, decided 2026-10-21): a
//! `dm_participant`-layout record owned by a random keypair only we hold,
//! carrying our `GroupContent` entries. It is made before the group (or our
//! accept) names it, single-flight per group, and its owner keypair is saved
//! to the vault under the group's id (a conversation, as a peer's key is
//! for a 1:1 chain) before the key leaves us.

use rekindle_transport::operations::dm::stream::ParticipantWriter;
use rekindle_transport::operations::friend::{FriendError, FriendHost};
use rekindle_vault::typed::owner_keys::{self, OwnerEntry};

use crate::host::adapters::friends::HostFriends;

fn saved(host: &HostFriends, group_id: &str) -> Result<Option<OwnerEntry>, FriendError> {
    let guard = host.ctx().vault.lock();
    let vault = guard.as_ref().ok_or(FriendError::Locked)?;
    owner_keys::load_participant_owner(vault, group_id)
        .map_err(|e| FriendError::Failed(format!("group chain owner not read: {e}")))
}

/// Our group chain for `group_id` (32 hex), made when none is saved.
///
/// # Errors
/// Locked, the network not ready, or the vault.
pub(crate) async fn writer(
    host: &HostFriends,
    group_id: &str,
) -> Result<ParticipantWriter, FriendError> {
    let gate = host.stores().participant_gate(group_id);
    let _one = gate.lock().await;
    if let Some(entry) = saved(host, group_id)? {
        return Ok(ParticipantWriter {
            record_key: entry.record_key,
            writer: entry.keypair,
            created: false,
        });
    }
    let pool = host
        .node()?
        .require_records()
        .map_err(|e| FriendError::NotReady(format!("records: {e}")))?;
    let (lease, record_key, keypair) = pool
        .create(&rekindle_records::schema::dm_participant(), None)
        .await
        .map_err(|e| FriendError::NotReady(format!("group chain record: {e}")))?;
    pool.release(lease).await;
    {
        let guard = host.ctx().vault.lock();
        let vault = guard.as_ref().ok_or(FriendError::Locked)?;
        let entry = OwnerEntry {
            record_key: record_key.clone(),
            keypair: keypair.clone(),
        };
        owner_keys::persist_participant_owner(vault, group_id, &entry)
            .map_err(|e| FriendError::Failed(format!("group chain owner not saved: {e}")))?;
    }
    tracing::info!(group = %group_id, "group chain record created");
    Ok(ParticipantWriter {
        record_key,
        writer: keypair,
        created: true,
    })
}
