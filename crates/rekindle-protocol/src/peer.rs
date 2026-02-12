use std::collections::HashMap;

use crate::error::ProtocolError;

/// Information about a known peer.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// Ed25519 public key (hex).
    pub public_key: String,
    /// Their profile DHT record key.
    pub profile_dht_key: Option<String>,
    /// Cached route blob for sending messages.
    pub route_blob: Option<Vec<u8>>,
    /// Whether we have an active Signal session.
    pub has_session: bool,
    /// Last time we saw this peer online.
    pub last_seen: Option<u64>,
}

/// Manages known peers and their routing information.
pub struct PeerManager {
    peers: HashMap<String, PeerInfo>,
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    /// Register a known peer.
    pub fn add_peer(&mut self, info: PeerInfo) {
        self.peers.insert(info.public_key.clone(), info);
    }

    /// Remove a peer.
    pub fn remove_peer(&mut self, public_key: &str) {
        self.peers.remove(public_key);
    }

    /// Get info about a peer.
    pub fn get_peer(&self, public_key: &str) -> Option<&PeerInfo> {
        self.peers.get(public_key)
    }

    /// Get mutable info about a peer.
    pub fn get_peer_mut(&mut self, public_key: &str) -> Option<&mut PeerInfo> {
        self.peers.get_mut(public_key)
    }

    /// Update a peer's route blob (called when their DHT subkey 6 changes).
    pub fn update_route(&mut self, public_key: &str, route_blob: Vec<u8>) {
        if let Some(peer) = self.peers.get_mut(public_key) {
            peer.route_blob = Some(route_blob);
        }
    }

    /// Get the route blob for a peer (for sending messages).
    pub fn get_route(&self, public_key: &str) -> Result<&[u8], ProtocolError> {
        self.peers
            .get(public_key)
            .and_then(|p| p.route_blob.as_deref())
            .ok_or_else(|| ProtocolError::Network(format!("no route for peer {public_key}")))
    }

    /// List all known peers.
    pub fn list_peers(&self) -> Vec<&PeerInfo> {
        self.peers.values().collect()
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new()
    }
}
