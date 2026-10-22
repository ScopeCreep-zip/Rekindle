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
mod tests {
    use std::sync::Arc;

    use rekindle_ipc::protocol::IpcRequest;
    use rekindle_ipc::{Caller, HostApi};

    use super::*;
    use crate::host::{claim, Host, HostConfig, HostStores, TransportSource};

    /// A host on one simulated node, with an identity unlocked and `peer`
    /// an accepted friend.
    async fn host_on_sim(
        dir: &tempfile::TempDir,
        peer: &str,
    ) -> (rekindle_netsim::Netsim, Arc<Host>) {
        let (sim, mut nodes) = rekindle_netsim::Netsim::nodes(1, 81);
        let node = nodes.remove(0);
        let data = dir.path().join("data");
        let root = rekindle_db::paths::DataRoot {
            config: dir.path().join("config"),
            logs: dir.path().join("logs"),
            state: data.join("state"),
            data,
        };
        let storage_dir = root.data.join("veilid").display().to_string();
        let host = Host::start(HostConfig {
            root: claim(root).unwrap(),
            stores: HostStores::default(),
            transport: TransportSource::Ports {
                config: rekindle_types::config::TransportConfig {
                    storage_dir,
                    ..Default::default()
                },
                ports: node.ports,
                events: node.events,
            },
            serve_bus: false,
            clock: Arc::clone(&node.clock) as Arc<dyn rekindle_utils::Clock>,
            rng: Arc::clone(&node.rng) as Arc<dyn rekindle_utils::RandomSource>,
            idle: Arc::clone(&node.idle) as Arc<dyn rekindle_utils::IdleSource>,
            shell: None,
        })
        .await
        .unwrap();
        let _pump = sim.start();
        sim.quiesce().await;
        let created = host
            .call(
                Caller::shell(),
                IpcRequest::IdentityCreate {
                    display_name: Some("Ann".into()),
                    passphrase: "pass".into(),
                },
            )
            .await;
        assert!(
            !matches!(created, rekindle_ipc::protocol::IpcResponse::Error { .. }),
            "{created:?}"
        );
        let owner = host.ctx.require_owner().unwrap();
        let p = peer.to_string();
        host.ctx
            .db
            .current()
            .unwrap()
            .call(move |conn| {
                rekindle_db::repo::friends::insert_accepted(conn, &owner, &p, "Bob", 1)
            })
            .await
            .unwrap();
        (sim, host)
    }

    /// Two concurrent first frames to one peer create one participant
    /// record: one creates it, the other awaits it and uses it.
    #[tokio::test]
    async fn concurrent_first_sends_create_one_record() {
        let dir = tempfile::tempdir().unwrap();
        let peer = hex::encode(rekindle_crypto::Identity::generate().public_key_bytes());
        let (_sim, host) = host_on_sim(&dir, &peer).await;
        let friends = HostFriends::of(&host.ctx);
        assert!(friends
            .held_participant_writer(&peer)
            .await
            .unwrap()
            .is_none());
        let (a, b) = tokio::join!(writer(&friends, &peer), writer(&friends, &peer));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.record_key, b.record_key);
        assert_eq!(usize::from(a.created) + usize::from(b.created), 1);
        let held = friends
            .held_participant_writer(&peer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(held.record_key, a.record_key);
        assert!(!held.created);
    }
}
