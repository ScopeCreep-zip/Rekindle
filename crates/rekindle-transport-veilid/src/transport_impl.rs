//! Transport trait implementation for the Veilid transport provider.
//!
//! Thin wrapper around `TransportNode` which owns all subsystems.
//! VeilidTransport delegates every trait method to the node's subsystems.
//! No subsystem construction here — all done in TransportNode::start().

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_types::transport::{
    Transport, TransportError, TransportResult,
    BroadcastReport, RecordSchema, WatchToken, OpenRecord,
    Durability, DeliveryReport,
};

use crate::broadcast::dht_writes;
use crate::broadcast::node::{self, TransportNode};

/// Veilid-backed Transport implementation.
/// Thin wrapper — all subsystems owned by TransportNode.
pub struct VeilidTransport {
    node: Arc<TransportNode>,
    watch_counter: AtomicU64,
    watch_registry: RwLock<HashMap<u64, (String, Vec<u32>)>>,
}

impl VeilidTransport {
    pub fn new(node: Arc<TransportNode>) -> Self {
        Self {
            node,
            watch_counter: AtomicU64::new(1),
            watch_registry: RwLock::new(HashMap::new()),
        }
    }

    pub fn node(&self) -> &Arc<TransportNode> { &self.node }
}

#[async_trait]
impl Transport for VeilidTransport {
    async fn start(&self) -> TransportResult<()> { Ok(()) }
    async fn shutdown(&self) -> TransportResult<()> { self.node.graceful_shutdown().await; Ok(()) }
    fn is_attached(&self) -> bool { self.node.is_ready() }

    async fn send_to_peer(&self, peer_key: &str, data: &[u8]) -> TransportResult<()> {
        let peers = self.node.peers();
        let route_blob = { peers.read().get_route(peer_key).map(<[u8]>::to_vec) };
        let Some(blob) = route_blob else {
            return Err(TransportError::PeerUnreachable { peer_key: format!("{}…", &peer_key[..12.min(peer_key.len())]) });
        };
        let target = self.node.import_route(&blob).map_err(|e| TransportError::SendFailed { reason: format!("{e}") })?;
        self.node.sender().send_raw(&target, data).await.map_err(TransportError::from)
    }

    async fn call_peer(&self, peer_key: &str, data: &[u8]) -> TransportResult<Vec<u8>> {
        let peers = self.node.peers();
        let route_blob = { peers.read().get_route(peer_key).map(<[u8]>::to_vec) };
        let Some(blob) = route_blob else {
            return Err(TransportError::PeerUnreachable { peer_key: format!("{}…", &peer_key[..12.min(peer_key.len())]) });
        };
        let target = self.node.import_route(&blob).map_err(|e| TransportError::SendFailed { reason: format!("{e}") })?;
        self.node.caller().call_raw(&target, data).await.map_err(TransportError::from)
    }

    async fn create_record(&self, schema: RecordSchema) -> TransportResult<(OpenRecord, Vec<u8>)> {
        match schema {
            RecordSchema::SingleWriter { subkey_count } => {
                let count = u16::try_from(subkey_count).map_err(|_| TransportError::Internal(format!("subkey_count {subkey_count} exceeds u16::MAX")))?;
                let (key, kp) = dht_writes::create_dflt(&self.node, count, None).await.map_err(TransportError::from)?;
                Ok((OpenRecord::new(key), kp.map(|k| node::serialize_keypair(&k)).unwrap_or_default()))
            }
            RecordSchema::MultiWriter { owner_subkeys, member_subkeys, member_keys } => {
                let members: Vec<veilid_core::DHTSchemaSMPLMember> = member_keys.iter()
                    .map(|pubkey| veilid_core::DHTSchemaSMPLMember {
                        m_key: veilid_core::BareMemberId::new(pubkey),
                        m_cnt: member_subkeys,
                    })
                    .collect();
                let (key, kp) = dht_writes::create_smpl(&self.node, owner_subkeys, members).await.map_err(TransportError::from)?;
                Ok((OpenRecord::new(key), kp.map(|k| node::serialize_keypair(&k)).unwrap_or_default()))
            }
        }
    }

    async fn open_record(&self, key: &str, writer: Option<&[u8]>) -> TransportResult<OpenRecord> {
        if let Some(kp_bytes) = writer {
            let kp = node::deserialize_keypair(kp_bytes).map_err(TransportError::from)?;
            dht_writes::open_writable(&self.node, key, kp).await.map_err(TransportError::from)?;
        } else {
            dht_writes::open_readonly(&self.node, key).await.map_err(TransportError::from)?;
        }
        Ok(OpenRecord::new(key.to_string()))
    }

    async fn write_record(&self, record: &OpenRecord, subkey: u32, data: &[u8], writer: Option<&[u8]>) -> TransportResult<()> {
        let key = record.key();
        let kp = writer.map(node::deserialize_keypair).transpose().map_err(TransportError::from)?;
        dht_writes::set(&self.node, key, subkey, data.to_vec(), kp).await.map(|_| ()).map_err(TransportError::from)
    }

    async fn read_record(&self, record: &OpenRecord, subkey: u32, force_refresh: bool) -> TransportResult<Option<Vec<u8>>> {
        let key = record.key();
        dht_writes::get(&self.node, key, subkey, force_refresh).await.map_err(TransportError::from)
    }

    async fn watch_record(&self, record: &OpenRecord, subkeys: &[u32]) -> TransportResult<WatchToken> {
        let key = record.key();
        let active = dht_writes::watch(&self.node, key, subkeys, None, None).await.map_err(TransportError::from)?;
        if !active { return Err(TransportError::Internal(format!("watch declined for {}…", &key[..12.min(key.len())]))); }
        let token_id = self.watch_counter.fetch_add(1, Ordering::Relaxed);
        self.watch_registry.write().insert(token_id, (key.to_string(), subkeys.to_vec()));
        Ok(WatchToken(token_id))
    }

    async fn cancel_watch(&self, token: WatchToken) -> TransportResult<()> {
        self.watch_registry.write().remove(&token.0);
        Ok(())
    }

    async fn inspect_record(&self, record: &OpenRecord, subkeys: &[u32]) -> TransportResult<rekindle_types::transport::InspectResult> {
        let key = record.key();
        let report = dht_writes::inspect(&self.node, key, Some(subkeys), veilid_core::DHTReportScope::UpdateGet).await.map_err(TransportError::from)?;
        Ok(rekindle_types::transport::InspectResult {
            local_seqs: report.local_seqs().iter().map(veilid_core::ValueSeqNum::to_option).collect(),
            network_seqs: report.network_seqs().iter().map(veilid_core::ValueSeqNum::to_option).collect(),
        })
    }

    async fn close_record(&self, record: OpenRecord) -> TransportResult<()> {
        let key = record.key();
        dht_writes::close(&self.node, key).await.map_err(TransportError::from)
    }

    async fn delete_record(&self, key: &str) -> TransportResult<()> {
        dht_writes::delete(&self.node, key).await.map_err(TransportError::from)
    }

    async fn allocate_route(&self) -> TransportResult<(String, Vec<u8>)> {
        self.node.allocate_route().await.map_err(TransportError::from)
    }

    fn route_blob(&self) -> Option<Vec<u8>> { self.node.routes().read().route_blob().map(<[u8]>::to_vec) }
    fn cache_peer_route(&self, peer_key: &str, route_blob: Vec<u8>) { self.node.peers().write().cache_route(peer_key, route_blob); }
    fn invalidate_peer_route(&self, peer_key: &str) { self.node.peers().write().invalidate_route(peer_key); }

    async fn import_route(&self, route_blob: &[u8]) -> TransportResult<String> {
        let target = self.node.import_route(route_blob).map_err(TransportError::from)?;
        Ok(format!("{:?}", target.route_id))
    }

    async fn broadcast(&self, community_id: &str, data: &[u8]) -> TransportResult<BroadcastReport> {
        let report = self.node.broadcast_mgr().broadcast_to_mesh(community_id, data).await;
        Ok(BroadcastReport { peers_sent: u32::try_from(report.delivered).unwrap_or(u32::MAX), peers_failed: u32::try_from(report.failures.len()).unwrap_or(u32::MAX) })
    }

    async fn join_mesh(&self, community_id: &str) -> TransportResult<()> { self.node.broadcast_mgr().register_mesh(community_id); Ok(()) }
    async fn leave_mesh(&self, community_id: &str) -> TransportResult<()> { self.node.broadcast_mgr().deregister_mesh(community_id); Ok(()) }

    fn upsert_mesh_peer(&self, community_id: &str, pseudonym: &str, route_blob: Vec<u8>, status: &str, now_secs: u64, my_pseudonym: &str) {
        self.node.broadcast_mgr().upsert_mesh_peer(community_id, pseudonym, route_blob, status, now_secs, my_pseudonym);
    }
    fn remove_mesh_peer(&self, community_id: &str, pseudonym: &str) { self.node.broadcast_mgr().remove_mesh_peer(community_id, pseudonym); }

    fn register_peer_profile(&self, peer_key: &str, profile_dht_key: &str) {
        self.node.resolver().register_peer(peer_key, profile_dht_key);
        self.node.resolver().register_profile_self(profile_dht_key);
    }

    async fn deliver(&self, peer_key: &str, data: &[u8], durability: Durability) -> DeliveryReport {
        self.node.delivery().deliver(peer_key, data, durability).await
    }

    async fn deliver_community(&self, community_id: &str, data: &[u8], durability: Durability) -> DeliveryReport {
        self.node.delivery().deliver_community(community_id, data, durability).await
    }

    async fn populate_mesh(&self, community_id: &str, my_pseudonym: &str, members: &[(String, Option<String>)]) -> u32 {
        self.node.mesh_manager().populate(community_id, my_pseudonym, members).await
    }

    async fn refresh_mesh(&self, community_id: &str, my_pseudonym: &str) {
        self.node.mesh_manager().refresh(community_id, my_pseudonym).await;
    }

    fn evict_stale_mesh_members(&self) { self.node.mesh_manager().evict_all_stale(); }

    async fn send_file(&self, peer_key: &str, path: &std::path::Path, media_type: &str) -> TransportResult<[u8; 16]> {
        self.node.bulk_sender().send_file(peer_key, path, media_type).await
            .map(|p| p.transfer_id)
            .map_err(|e| TransportError::Internal(format!("{e}")))
    }

    fn cancel_transfer(&self, transfer_id: &[u8; 16]) { self.node.transfer_registry().cancel(transfer_id); }
    fn get_transfer_progress(&self, transfer_id: &[u8; 16]) -> Option<rekindle_types::transport::TransferProgress> {
        self.node.transfer_registry().transfer_progress(transfer_id)
    }
    fn active_transfers(&self) -> Vec<rekindle_types::transport::TransferProgress> {
        self.node.transfer_registry().active_transfers()
    }

    fn peer_count(&self) -> u32 { u32::try_from(self.node.status_snapshot().peer_count).unwrap_or(u32::MAX) }
    fn attachment_state(&self) -> &str { if self.node.is_ready() { "attached" } else { "detached" } }
    fn uptime_secs(&self) -> u64 { self.node.uptime().as_secs() }
    fn is_public_internet_ready(&self) -> bool { self.node.shared().public_internet_ready() }
    fn route_age_secs(&self) -> Option<u64> { self.node.routes().read().route_age().map(|d| d.as_secs()) }
    fn circuit_summary(&self) -> (usize, usize, usize, usize) {
        let s = self.node.peers().read().circuit_summary();
        (s.total, s.healthy, s.degraded, s.circuit_open)
    }
    fn gossip_mesh_peer_count(&self) -> usize {
        self.node.broadcast_mgr().meshes().read().values().map(|m| m.peers.len()).sum()
    }
}
