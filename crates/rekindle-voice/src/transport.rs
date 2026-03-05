use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use veilid_core::{RoutingContext, SafetySelection, Sequencing, Target, VeilidAPI};

use crate::codec::EncodedFrame;
use crate::error::VoiceError;

/// Voice packet for network transmission.
#[derive(Clone, Serialize, Deserialize)]
pub struct VoicePacket {
    /// Sender public key (32 bytes).
    pub sender_key: Vec<u8>,
    /// Sequence number for ordering.
    pub sequence: u32,
    /// Timestamp in milliseconds.
    pub timestamp: u64,
    /// Opus-encoded audio data.
    pub audio_data: Vec<u8>,
}

/// Voice channel operating mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceMode {
    /// Full mesh: each participant sends to every other.
    /// Suitable for 2-5 participants.
    #[default]
    Mesh,
    /// MCU mode: one peer (host) receives all, mixes, redistributes.
    /// Suitable for 6-15+ participants.
    Mcu {
        /// Pseudonym key (hex) of the mixing host.
        host_pseudonym: String,
    },
}

/// Per-peer routing info for multi-peer voice transport.
struct PeerRoute {
    routing_context: RoutingContext,
    route_id: veilid_core::RouteId,
}

/// Multi-peer voice transport over the Veilid network.
///
/// Supports both full-mesh (each peer sends to every other) and MCU
/// (all peers send to one host, host mixes and redistributes) modes.
///
/// Uses `SafetySelection::Unsafe` for voice to minimize latency,
/// trading sender privacy for acceptable voice quality.
pub struct VoiceTransport {
    channel_id: String,
    api: Option<VeilidAPI>,
    sender_key: Vec<u8>,
    /// Connected peers: pseudonym_key (hex) → route info.
    peers: HashMap<String, PeerRoute>,
    /// Current operating mode.
    mode: VoiceMode,
}

impl VoiceTransport {
    /// Create a new transport for a voice channel.
    pub fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            api: None,
            sender_key: Vec::new(),
            peers: HashMap::new(),
            mode: VoiceMode::default(),
        }
    }

    /// Initialize the transport with a Veilid API and sender identity.
    ///
    /// This replaces the old single-peer `connect()` for backward compatibility.
    /// After calling `init()`, add peers with `add_peer()`.
    pub fn init(&mut self, api: VeilidAPI, sender_key: Vec<u8>) {
        self.api = Some(api);
        self.sender_key = sender_key;
    }

    /// Legacy single-peer connect (wraps `init` + `add_peer`).
    ///
    /// Kept for backward compatibility with 1:1 DM voice calls.
    /// Uses `"default"` as the peer key for the single remote participant.
    pub fn connect(
        &mut self,
        api: VeilidAPI,
        route_blob: &[u8],
        sender_key: Vec<u8>,
    ) -> Result<(), VoiceError> {
        self.init(api, sender_key);
        self.add_peer("default", route_blob)?;
        tracing::info!(channel = %self.channel_id, "voice transport connected (legacy single-peer)");
        Ok(())
    }

    /// Add a peer to the voice mesh.
    pub fn add_peer(
        &mut self,
        pseudonym_key: &str,
        route_blob: &[u8],
    ) -> Result<(), VoiceError> {
        let api = self
            .api
            .as_ref()
            .ok_or_else(|| VoiceError::Transport("transport not initialized".into()))?;

        let routing_context = api
            .routing_context()
            .map_err(|e| VoiceError::Transport(format!("routing context: {e}")))?
            .with_safety(SafetySelection::Unsafe(Sequencing::NoPreference))
            .map_err(|e| VoiceError::Transport(format!("with_safety: {e}")))?;

        let route_id = api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| VoiceError::Transport(format!("import route: {e}")))?;

        tracing::info!(
            channel = %self.channel_id,
            peer = %pseudonym_key,
            "added voice peer"
        );

        self.peers.insert(
            pseudonym_key.to_string(),
            PeerRoute {
                routing_context,
                route_id,
            },
        );

        Ok(())
    }

    /// Remove a peer from the voice mesh.
    pub fn remove_peer(&mut self, pseudonym_key: &str) {
        if self.peers.remove(pseudonym_key).is_some() {
            tracing::info!(
                channel = %self.channel_id,
                peer = %pseudonym_key,
                "removed voice peer"
            );
        }
    }

    /// Set the voice channel operating mode.
    pub fn set_mode(&mut self, mode: VoiceMode) {
        tracing::info!(channel = %self.channel_id, ?mode, "voice mode changed");
        self.mode = mode;
    }

    /// Get the current voice mode.
    pub fn mode(&self) -> &VoiceMode {
        &self.mode
    }

    /// Number of connected peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Broadcast an encoded audio frame to ALL connected peers (mesh mode).
    ///
    /// Returns a list of (pseudonym_key, error) for any failed sends.
    pub async fn broadcast(&self, frame: &EncodedFrame) -> Vec<(String, VoiceError)> {
        let data = match self.build_packet_data(frame) {
            Ok(d) => d,
            Err(e) => return vec![("*".into(), e)],
        };

        let mut errors = Vec::new();
        for (key, peer) in &self.peers {
            if let Err(e) = peer
                .routing_context
                .app_message(Target::RouteId(peer.route_id.clone()), data.clone())
                .await
            {
                errors.push((key.clone(), VoiceError::Transport(format!("app_message: {e}"))));
            }
        }
        errors
    }

    /// Send an encoded audio frame to a specific peer (MCU mode).
    pub async fn send_to_peer(
        &self,
        pseudonym_key: &str,
        frame: &EncodedFrame,
    ) -> Result<(), VoiceError> {
        let peer = self.peers.get(pseudonym_key).ok_or_else(|| {
            VoiceError::Transport(format!("peer not found: {pseudonym_key}"))
        })?;

        let data = self.build_packet_data(frame)?;

        peer.routing_context
            .app_message(Target::RouteId(peer.route_id.clone()), data)
            .await
            .map_err(|e| VoiceError::Transport(format!("app_message: {e}")))?;

        Ok(())
    }

    /// Legacy single-peer send (broadcasts to all peers).
    ///
    /// Kept for backward compatibility — the send loop calls this.
    pub async fn send(&self, frame: &EncodedFrame) -> Result<(), VoiceError> {
        if self.peers.is_empty() {
            return Err(VoiceError::NotConnected);
        }

        let errors = self.broadcast(frame).await;
        if errors.len() == self.peers.len() {
            // All sends failed — report the first error
            return Err(errors.into_iter().next().map_or(VoiceError::NotConnected, |(_, e)| e));
        }
        Ok(())
    }

    /// Disconnect from the voice channel — removes all peers.
    pub fn disconnect(&mut self) {
        self.peers.clear();
        self.api = None;
        self.sender_key.clear();
        self.mode = VoiceMode::default();
        tracing::info!(channel = %self.channel_id, "voice transport disconnected");
    }

    /// Deserialize an incoming voice packet from raw bytes.
    ///
    /// Expects data WITHOUT the `b'V'` type tag prefix — the dispatch loop
    /// should strip the tag before calling this.
    pub fn receive(data: &[u8]) -> Result<VoicePacket, VoiceError> {
        bincode::deserialize(data).map_err(|e| VoiceError::Transport(format!("{e}")))
    }

    /// Whether the transport has any connected peers.
    pub fn is_connected(&self) -> bool {
        !self.peers.is_empty()
    }

    /// The channel ID this transport is for.
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    /// Build the wire-format packet data from an encoded frame.
    fn build_packet_data(&self, frame: &EncodedFrame) -> Result<Vec<u8>, VoiceError> {
        let packet = VoicePacket {
            sender_key: self.sender_key.clone(),
            sequence: frame.sequence,
            timestamp: frame.timestamp,
            audio_data: frame.data.clone(),
        };

        let payload =
            bincode::serialize(&packet).map_err(|e| VoiceError::Transport(format!("{e}")))?;

        // Prepend voice type tag (b'V') so the dispatch loop can distinguish
        // voice packets from chat messages and community broadcasts.
        let mut data = Vec::with_capacity(1 + payload.len());
        data.push(b'V');
        data.extend_from_slice(&payload);
        Ok(data)
    }
}
