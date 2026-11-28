//! Phase 16 — VideoDeps trait + VideoFlowEvent enum.
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
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_types::id::PseudonymKey;
use rekindle_types::video::{Codec, ScalabilityMode, TopologyReason};

/// Events emitted to the UI from receive-side flows. Each variant
/// maps 1:1 to a `CommunityEvent::Video*` shape; the adapter does the
/// translation.
#[derive(Debug, Clone)]
pub enum VideoFlowEvent {
    /// A complete frame is ready for the webview decoder (plaintext
    /// payload — MEK-decryption is the crate's responsibility).
    FrameReady {
        community_id: String,
        sender_pseudonym: String,
        stream_id: [u8; 16],
        frame_seq: u32,
        keyframe: bool,
        /// Codec the frame was encoded with — the frontend configures
        /// its decoder from this tag, never from the session config.
        codec: rekindle_types::video::Codec,
        timestamp: u32,
        payload: Vec<u8>,
    },
    KeyframeRequest {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        stream_id: [u8; 16],
    },
    TopologyChange {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        stream_id: [u8; 16],
        relay_host_pseudonym: Option<String>,
        reason: TopologyReason,
        lamport: u64,
    },
    MediaCapabilities {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        max_pixel_count: u32,
        max_fps: u8,
        encode_codecs: Vec<Codec>,
        decode_codecs: Vec<Codec>,
        supports_optimize_for_latency: bool,
        supported_scalability_modes: Vec<ScalabilityMode>,
    },
    /// Phase F — a video fragment / parity / capabilities envelope was
    /// rejected at the gossip-verify boundary. The frontend renders this
    /// as a UI hint so users see asymmetric-drop conditions without grep.
    ///
    /// Routing fields may be absent if the envelope did not decode.
    EnvelopeRejected {
        community_id: Option<String>,
        sender_pseudonym: Option<PseudonymKey>,
    },
}

/// Single deps trait for the community-video flows. The reassembly
/// buffer is NOT exposed through the trait — pass it as a parameter
/// to crate-side fns instead.
pub trait VideoDeps: Send + Sync + 'static {
    /// The sender keys of our session on `(community, channel)` (plan
    /// C7.20), the same store voice uses: frames are sealed under our own
    /// media key and opened under each sender's.
    fn channel_sender_keys(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> std::sync::Arc<rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys>;

    /// Derive the Ed25519 SigningKey for the community pseudonym (the
    /// fragment-level signature uses this). Returns `None` if the
    /// identity secret isn't unlocked.
    fn community_signing_key(
        &self,
        community_id: &str,
    ) -> Option<rekindle_secrets::ed25519_dalek::SigningKey>;

    /// Send an envelope to exactly the peers in the given voice/video
    /// channel's roster (directed `app_message` per peer, ttl = 0) —
    /// never to the community gossip mesh. Architecture §10.6: media
    /// and its per-stream control traffic stay inside the channel the
    /// members are actively in.
    fn send_to_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), VideoError>;

    /// The channel the local user is actively joined to in this
    /// community's voice/video session, or `None` when not in any
    /// channel of this community. Reader-validates gate for every
    /// inbound video payload: anything addressed to a different
    /// channel is dropped before reassembly / decrypt / emit.
    fn local_active_channel(&self, community_id: &str) -> Option<String>;

    /// Next message-clock value. Used by `TopologyChange` writes so
    /// lamport-LWW dedup works at every receiver.
    fn increment_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError>;

    /// Ask `sender` (pseudonym hex) for its media key at `index`
    /// (`VoiceMediaKeyRequest`, plan C7.20): a reassembled frame arrived
    /// under a key it has not sent us, or whose push was lost. Silence
    /// would be a permanently black tile. Debounced by the caller;
    /// fire-and-forget.
    fn request_media_key(&self, community_id: &str, channel_id: &str, sender: &str, index: u64);

    /// Emit a UI-facing event from a receive-side handler.
    fn emit_event(&self, event: VideoFlowEvent);

    /// Reassembly completed frame `frame_seq` of `stream_id` from `sender`
    /// (pseudonym hex), with parity if `recovered_via_fec`: the post-FEC
    /// loss measurement (plan E4.3 Q0).
    fn note_frame_completed(
        &self,
        stream_id: [u8; 16],
        sender: &str,
        frame_seq: u32,
        recovered_via_fec: bool,
    );
}
