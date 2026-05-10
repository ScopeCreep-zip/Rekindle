//! Transport trait implementation for the Veilid transport provider.
//!
//! Wraps `TransportNode` + `BroadcastManager` and delegates every trait
//! method to the corresponding Veilid operation. All data is opaque bytes —
//! no payload inspection, no signing, no deserialization.
//!
//! Error mapping: `crate::error::TransportError` → `rekindle_types::transport::TransportError`.
//! Every error preserves the original message for actionable diagnostics.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_types::transport::{
    Transport, TransportCallback, TransportError, TransportResult,
    BroadcastReport, RecordSchema, WatchToken,
};

use crate::broadcast::node::TransportNode;
use crate::broadcast::BroadcastManager;

/// Veilid-backed Transport implementation.
///
/// Holds the TransportNode (Veilid API lifecycle), BroadcastManager (gossip
/// mesh state + fan-out), and a watch token registry for cancel_watch support.
pub struct VeilidTransport {
    node: Arc<TransportNode>,
    broadcast: Arc<BroadcastManager>,
    watch_counter: AtomicU64,
    watch_registry: RwLock<HashMap<u64, (String, Vec<u32>)>>,
}

impl VeilidTransport {
    pub fn new(node: Arc<TransportNode>) -> Self {
        let broadcast = Arc::new(BroadcastManager::new(Arc::clone(&node)));
        Self {
            node,
            broadcast,
            watch_counter: AtomicU64::new(1),
            watch_registry: RwLock::new(HashMap::new()),
        }
    }

    pub fn node(&self) -> &Arc<TransportNode> {
        &self.node
    }

    pub fn broadcast_manager(&self) -> &Arc<BroadcastManager> {
        &self.broadcast
    }
}

#[async_trait]
impl Transport for VeilidTransport {
    // ── Lifecycle ───────────────────────────────────────────────

    async fn start(&self) -> TransportResult<()> {
        // TransportNode is started at construction time via TransportNode::start().
        // This method exists for backends that defer startup.
        Ok(())
    }

    async fn shutdown(&self) -> TransportResult<()> {
        self.node.graceful_shutdown().await;
        Ok(())
    }

    fn set_callback(&self, callback: Arc<dyn TransportCallback>) {
        self.node.set_callback(callback);
    }

    fn is_attached(&self) -> bool {
        self.node.is_ready()
    }

    // ── Peer messaging ──────────────────────────────────────────

    async fn send_to_peer(&self, peer_key: &str, data: &[u8]) -> TransportResult<()> {
        // Bind the Arc to a variable before calling .read() so the Arc
        // outlives the RwLockReadGuard. Without this binding, the Arc is
        // a temporary that's dropped at the semicolon while the guard
        // still borrows it.
        let peers = self.node.peers();
        let route_blob = {
            let registry = peers.read();
            registry.get_route(peer_key).map(<[u8]>::to_vec)
        };
        let Some(blob) = route_blob else {
            return Err(TransportError::PeerUnreachable {
                peer_key: format!(
                    "{}… — no cached route. Call cache_peer_route first.",
                    &peer_key[..12.min(peer_key.len())]
                ),
            });
        };

        let target = self.node.import_route(&blob).map_err(|e| {
            TransportError::SendFailed {
                reason: format!(
                    "route import failed for peer {}…: {e}",
                    &peer_key[..12.min(peer_key.len())]
                ),
            }
        })?;

        self.node.sender().send_raw(&target, data).await.map_err(rekindle_types::transport::TransportError::from)
    }

    async fn call_peer(&self, peer_key: &str, data: &[u8]) -> TransportResult<Vec<u8>> {
        let peers = self.node.peers();
        let route_blob = {
            let registry = peers.read();
            registry.get_route(peer_key).map(<[u8]>::to_vec)
        };
        let Some(blob) = route_blob else {
            return Err(TransportError::PeerUnreachable {
                peer_key: format!(
                    "{}… — no cached route",
                    &peer_key[..12.min(peer_key.len())]
                ),
            });
        };

        let target = self.node.import_route(&blob).map_err(|e| {
            TransportError::SendFailed {
                reason: format!("route import for call: {e}"),
            }
        })?;

        self.node.caller().call_raw(&target, data).await.map_err(rekindle_types::transport::TransportError::from)
    }

    // ── Persistent records ──────────────────────────────────────

    async fn create_record(&self, schema: RecordSchema) -> TransportResult<(String, Vec<u8>)> {
        match schema {
            RecordSchema::SingleWriter { subkey_count } => {
                let count = u16::try_from(subkey_count).map_err(|_| {
                    TransportError::Internal(format!(
                        "subkey_count {subkey_count} exceeds u16::MAX for DFLT schema"
                    ))
                })?;
                let (key, kp) = crate::broadcast::dht_writes::create_dflt(
                    &self.node, count, None,
                ).await.map_err(rekindle_types::transport::TransportError::from)?;
                let keypair_bytes = kp
                    .map(|k| crate::broadcast::node::serialize_keypair(&k))
                    .unwrap_or_default();
                Ok((key, keypair_bytes))
            }
            RecordSchema::MultiWriter { owner_subkeys, member_subkeys, member_count } => {
                // Construct SMPL member descriptors with unassigned (default) member IDs.
                // BareMemberId::default() produces an empty key — Veilid treats this as
                // an unassigned slot that any writer can claim by opening the record
                // with their keypair.
                let members: Vec<veilid_core::DHTSchemaSMPLMember> = (0..member_count)
                    .map(|_| veilid_core::DHTSchemaSMPLMember {
                        m_key: veilid_core::BareMemberId::default(),
                        m_cnt: member_subkeys,
                    })
                    .collect();
                let (key, kp) = crate::broadcast::dht_writes::create_smpl(
                    &self.node, owner_subkeys, members,
                ).await.map_err(rekindle_types::transport::TransportError::from)?;
                let keypair_bytes = kp
                    .map(|k| crate::broadcast::node::serialize_keypair(&k))
                    .unwrap_or_default();
                Ok((key, keypair_bytes))
            }
        }
    }

    async fn open_record(&self, key: &str, writer: Option<&[u8]>) -> TransportResult<()> {
        if let Some(kp_bytes) = writer {
            let kp = crate::broadcast::node::deserialize_keypair(kp_bytes).map_err(rekindle_types::transport::TransportError::from)?;
            crate::broadcast::dht_writes::open_writable(&self.node, key, kp)
                .await.map_err(rekindle_types::transport::TransportError::from)
        } else {
            crate::broadcast::dht_writes::open_readonly(&self.node, key)
                .await.map_err(rekindle_types::transport::TransportError::from)
        }
    }

    async fn write_record(
        &self, key: &str, subkey: u32, data: &[u8], writer: Option<&[u8]>,
    ) -> TransportResult<()> {
        let kp = writer
            .map(crate::broadcast::node::deserialize_keypair)
            .transpose()
            .map_err(rekindle_types::transport::TransportError::from)?;
        crate::broadcast::dht_writes::set(
            &self.node, key, subkey, data.to_vec(), kp,
        ).await.map_err(rekindle_types::transport::TransportError::from)
    }

    async fn read_record(
        &self, key: &str, subkey: u32, force_refresh: bool,
    ) -> TransportResult<Option<Vec<u8>>> {
        crate::broadcast::dht_writes::get(&self.node, key, subkey, force_refresh)
            .await.map_err(rekindle_types::transport::TransportError::from)
    }

    async fn watch_record(
        &self, key: &str, subkeys: &[u32],
    ) -> TransportResult<WatchToken> {
        let active = crate::broadcast::dht_writes::watch(&self.node, key, subkeys)
            .await.map_err(rekindle_types::transport::TransportError::from)?;

        if !active {
            return Err(TransportError::Internal(format!(
                "watch declined by Veilid for record {}… — \
                 the node may have reached its watch limit. \
                 Reduce watch count or increase Veilid's public_watch_limit.",
                &key[..12.min(key.len())]
            )));
        }

        let token_id = self.watch_counter.fetch_add(1, Ordering::Relaxed);
        self.watch_registry.write().insert(
            token_id,
            (key.to_string(), subkeys.to_vec()),
        );

        Ok(WatchToken(token_id))
    }

    async fn cancel_watch(&self, token: WatchToken) -> TransportResult<()> {
        let entry = self.watch_registry.write().remove(&token.0);
        let Some((record_key, subkeys)) = entry else {
            tracing::debug!(
                token = token.0,
                "cancel_watch: token not found — watch may have already expired"
            );
            return Ok(());
        };

        tracing::debug!(
            token = token.0,
            record_key = &record_key[..12.min(record_key.len())],
            subkeys = subkeys.len(),
            "watch cancelled (will expire at Veilid TTL)"
        );

        Ok(())
    }

    async fn inspect_record(
        &self, key: &str, subkeys: &[u32],
    ) -> TransportResult<Vec<Option<u32>>> {
        let report = crate::broadcast::dht_writes::inspect(
            &self.node, key, Some(subkeys),
        ).await.map_err(rekindle_types::transport::TransportError::from)?;

        // ValueSeqNum wraps Option<u32>. Use .to_option() to extract.
        // None means no value written to that subkey.
        // Some(n) means the subkey has been written n times.
        let seqs: Vec<Option<u32>> = report.local_seqs()
            .iter()
            .map(veilid_core::ValueSeqNum::to_option)
            .collect();

        Ok(seqs)
    }

    async fn close_record(&self, key: &str) -> TransportResult<()> {
        crate::broadcast::dht_writes::close(&self.node, key).await.map_err(rekindle_types::transport::TransportError::from)
    }

    // ── Route management ────────────────────────────────────────

    async fn allocate_route(&self) -> TransportResult<(String, Vec<u8>)> {
        self.node.allocate_route().await.map_err(rekindle_types::transport::TransportError::from)
    }

    fn route_blob(&self) -> Option<Vec<u8>> {
        let routes = self.node.routes();
        let guard = routes.read();
        guard.route_blob().map(<[u8]>::to_vec)
    }

    fn cache_peer_route(&self, peer_key: &str, route_blob: Vec<u8>) {
        self.node.peers().write().cache_route(peer_key, route_blob);
    }

    fn invalidate_peer_route(&self, peer_key: &str) {
        self.node.peers().write().invalidate_route(peer_key);
    }

    async fn import_route(&self, route_blob: &[u8]) -> TransportResult<String> {
        let target = self.node.import_route(route_blob).map_err(rekindle_types::transport::TransportError::from)?;
        Ok(format!("{:?}", target.route_id))
    }

    // ── Community broadcast ─────────────────────────────────────

    async fn broadcast(
        &self, community_id: &str, data: &[u8],
    ) -> TransportResult<BroadcastReport> {
        let report = self.broadcast.broadcast_to_mesh(community_id, data).await;
        Ok(BroadcastReport {
            peers_sent: u32::try_from(report.delivered).unwrap_or(u32::MAX),
            peers_failed: u32::try_from(report.failures.len()).unwrap_or(u32::MAX),
        })
    }

    async fn join_mesh(&self, community_id: &str) -> TransportResult<()> {
        self.broadcast.register_mesh(community_id);
        Ok(())
    }

    async fn leave_mesh(&self, community_id: &str) -> TransportResult<()> {
        self.broadcast.deregister_mesh(community_id);
        Ok(())
    }

    // ── Diagnostics ─────────────────────────────────────────────

    fn peer_count(&self) -> u32 {
        let snapshot = self.node.status_snapshot();
        u32::try_from(snapshot.peer_count).unwrap_or(u32::MAX)
    }

    fn attachment_state(&self) -> &str {
        if self.node.is_ready() { "attached" } else { "detached" }
    }

    fn uptime_secs(&self) -> u64 {
        self.node.uptime().as_secs()
    }
}
