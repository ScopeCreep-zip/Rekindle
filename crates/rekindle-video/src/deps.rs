//! Phase 16 — VideoDeps trait + VideoEvent enum.
//!
//! The community-video send / receive / dispatch flows parameterise
//! over `VideoDeps` so the crate-side bodies never import
//! `veilid-core` or `tauri` directly (Invariant 2). The src-tauri
//! `VideoAdapter` (task #141) supplies the live wiring.
//!
//! Design note: the per-stream reassembly buffer (`VideoReassemblyState`,
//! task #138) is a concrete type that will live in this crate; the
//! crate-side bodies pass a `&VideoReassemblyState` parameter rather
//! than abstracting it through a trait method. Keeps the trait
//! surface small (~8 methods) and avoids unnecessary indirection on
//! the hot path.

use crate::error::VideoError;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

/// Events emitted to the UI from receive-side flows. Each variant
/// maps 1:1 to a `CommunityEvent::Video*` shape; the adapter does the
/// translation.
#[derive(Debug, Clone)]
pub enum VideoEvent {
    /// A complete frame is ready for the webview decoder (plaintext
    /// payload — MEK-decryption is the crate's responsibility).
    FrameReady {
        community_id: String,
        sender_pseudonym: String,
        stream_id: [u8; 16],
        frame_seq: u32,
        keyframe: bool,
        timestamp: u32,
        payload: Vec<u8>,
    },
    FrameAck {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        stream_id: [u8; 16],
        last_frame_seq: u32,
        kbps: u32,
        loss_q8: u8,
    },
    KeyframeRequest {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        stream_id: [u8; 16],
    },
    BandwidthEstimate {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        kbps: u32,
        window_secs: u8,
        loss_q8: u8,
    },
    TopologyChange {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        stream_id: [u8; 16],
        relay_host_pseudonym: Option<String>,
        reason: String,
        lamport: u64,
    },
    MediaCapabilities {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        max_pixel_count: u32,
        max_fps: u8,
        codecs: Vec<String>,
    },
}

/// Single deps trait for the community-video flows. The reassembly
/// buffer is NOT exposed through the trait — pass it as a parameter
/// to crate-side fns instead.
pub trait VideoDeps: Send + Sync + 'static {
    /// Current per-community MEK (raw 32 bytes + generation) for
    /// envelope encrypt/decrypt. Returns `None` if no MEK is cached
    /// (e.g. the user hasn't joined voice/video for this community).
    fn community_mek_bytes(&self, community_id: &str) -> Option<([u8; 32], u64)>;

    /// Derive the Ed25519 SigningKey for the community pseudonym (the
    /// fragment-level signature uses this). Returns `None` if the
    /// identity secret isn't unlocked.
    fn community_signing_key(
        &self,
        community_id: &str,
    ) -> Option<rekindle_secrets::ed25519_dalek::SigningKey>;

    /// Broadcast a gossip envelope to the community mesh.
    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), VideoError>;

    /// Increment the per-community Lamport clock and return the new
    /// value. Used by `TopologyChange` writes so lamport-LWW dedup
    /// works at every receiver.
    fn increment_lamport(&self, community_id: &str) -> u64;

    /// Emit a UI-facing event from a receive-side handler.
    fn emit_event(&self, event: VideoEvent);
}
