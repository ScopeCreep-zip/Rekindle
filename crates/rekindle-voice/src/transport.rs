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

/// Voice transport over the Veilid network.
///
/// Uses `SafetySelection::Unsafe` for voice to minimize latency,
/// trading sender privacy for acceptable voice quality.
pub struct VoiceTransport {
    channel_id: String,
    is_connected: bool,
    api: Option<VeilidAPI>,
    routing_context: Option<RoutingContext>,
    route_id: Option<veilid_core::RouteId>,
    sender_key: Vec<u8>,
}

impl VoiceTransport {
    /// Create a new transport for a voice channel.
    pub fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            is_connected: false,
            api: None,
            routing_context: None,
            route_id: None,
            sender_key: Vec::new(),
        }
    }

    /// Connect to the voice channel's routing context.
    pub fn connect(
        &mut self,
        api: VeilidAPI,
        route_blob: &[u8],
        sender_key: Vec<u8>,
    ) -> Result<(), VoiceError> {
        let routing_context = api
            .routing_context()
            .map_err(|e| VoiceError::Transport(format!("routing context: {e}")))?
            .with_safety(SafetySelection::Unsafe(Sequencing::NoPreference))
            .map_err(|e| VoiceError::Transport(format!("with_safety: {e}")))?;

        let route_id = api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| VoiceError::Transport(format!("import route: {e}")))?;

        self.api = Some(api);
        self.routing_context = Some(routing_context);
        self.route_id = Some(route_id);
        self.sender_key = sender_key;
        self.is_connected = true;
        tracing::info!(channel = %self.channel_id, "voice transport connected");
        Ok(())
    }

    /// Disconnect from the voice channel.
    pub fn disconnect(&mut self) -> Result<(), VoiceError> {
        self.route_id = None;
        self.routing_context = None;
        self.api = None;
        self.is_connected = false;
        tracing::info!(channel = %self.channel_id, "voice transport disconnected");
        Ok(())
    }

    /// Send an encoded audio frame to the remote participant.
    pub async fn send(&self, frame: &EncodedFrame) -> Result<(), VoiceError> {
        if !self.is_connected {
            return Err(VoiceError::NotConnected);
        }

        let routing_context = self
            .routing_context
            .as_ref()
            .ok_or(VoiceError::NotConnected)?;

        let route_id = self
            .route_id
            .clone()
            .ok_or(VoiceError::NotConnected)?;

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

        routing_context
            .app_message(Target::RouteId(route_id), data)
            .await
            .map_err(|e| VoiceError::Transport(format!("app_message: {e}")))?;

        Ok(())
    }

    /// Deserialize an incoming voice packet from raw bytes.
    ///
    /// Expects data WITHOUT the `b'V'` type tag prefix — the dispatch loop
    /// should strip the tag before calling this.
    pub fn receive(data: &[u8]) -> Result<VoicePacket, VoiceError> {
        bincode::deserialize(data).map_err(|e| VoiceError::Transport(format!("{e}")))
    }

    /// Whether the transport is connected.
    pub fn is_connected(&self) -> bool {
        self.is_connected
    }

    /// The channel ID this transport is for.
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }
}
