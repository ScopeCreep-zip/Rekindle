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
use rekindle_types::video::{Codec, ScalabilityMode};

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
        /// Codec the frame was encoded with — the frontend configures
        /// its decoder from this tag, never from the session config.
        codec: rekindle_types::video::Codec,
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
        encode_codecs: Vec<Codec>,
        decode_codecs: Vec<Codec>,
        supports_optimize_for_latency: bool,
        supported_scalability_modes: Vec<ScalabilityMode>,
    },
    /// Phase F — a video fragment / parity / capabilities envelope was
    /// rejected at the gossip-verify boundary. The frontend renders this
    /// as a UI hint so users see asymmetric-drop conditions without grep.
    ///
    /// **Sentinel values:** if the envelope failed to deserialize before
    /// we could read the routing fields, the emitter substitutes
    /// `"<unknown>"` literally for `community_id` and/or
    /// `sender_pseudonym`. Callers must treat that exact string as the
    /// "indeterminate" marker — no other fallback is permitted.
    EnvelopeRejected {
        community_id: String,
        sender_pseudonym: String,
        reason: String,
    },
}

/// Single deps trait for the community-video flows. The reassembly
/// buffer is NOT exposed through the trait — pass it as a parameter
/// to crate-side fns instead.
pub trait VideoDeps: Send + Sync + 'static {
    /// The channel-media MEK (raw 32 bytes + generation) for envelope
    /// encrypt/decrypt — §10.5 hierarchy: the per-channel MEK when the
    /// join/leave rotation has distributed one, the community MEK
    /// otherwise (stage channels never rotate and so resolve to the
    /// community key). `None` if neither is cached.
    fn channel_media_mek(&self, community_id: &str, channel_id: &str)
        -> Option<([u8; 32], u64)>;

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

    /// Increment the per-community Lamport clock and return the new
    /// value. Used by `TopologyChange` writes so lamport-LWW dedup
    /// works at every receiver.
    fn increment_lamport(&self, community_id: &str) -> u64;

    /// A reassembled frame failed to decrypt under our current MEK —
    /// the sender is on a newer generation (voice MEK rotates on every
    /// membership change, §10.7), classically right after WE joined.
    /// Fire the RequestMEK cascade instead of dropping silently
    /// (silent drop = permanently black tile). Debounced by the
    /// caller; fire-and-forget.
    /// Fire the RequestMEK cascade naming the EXACT generation needed
    /// (from the undecryptable frame's wire field). `0` = "send me your
    /// current generation" (used at session join when nothing is
    /// cached).
    fn request_mek_refresh(&self, community_id: &str, channel_id: &str, needed_generation: u64);

    /// Emit a UI-facing event from a receive-side handler.
    fn emit_event(&self, event: VideoEvent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mock::MockDeps;

    /// Phase F — round-trip `EnvelopeRejected` through the MockDeps
    /// channel to confirm the wire shape (community_id, sender_pseudonym,
    /// reason) is preserved verbatim AND that the `"<unknown>"` sentinel
    /// values survive a Clone / Debug round-trip without being normalised
    /// away by a stray default.
    #[test]
    fn envelope_rejected_preserves_fields_and_sentinels() {
        let deps = MockDeps::new();
        deps.emit_event(VideoEvent::EnvelopeRejected {
            community_id: "community_abc".to_string(),
            sender_pseudonym: "deadbeef".to_string(),
            reason: "bad signature".to_string(),
        });
        // Sentinel-bearing variant.
        deps.emit_event(VideoEvent::EnvelopeRejected {
            community_id: "<unknown>".to_string(),
            sender_pseudonym: "<unknown>".to_string(),
            reason: "deserialize failed".to_string(),
        });

        let calls = deps.calls.lock();
        assert_eq!(calls.events.len(), 2);

        match &calls.events[0] {
            VideoEvent::EnvelopeRejected {
                community_id,
                sender_pseudonym,
                reason,
            } => {
                assert_eq!(community_id, "community_abc");
                assert_eq!(sender_pseudonym, "deadbeef");
                assert_eq!(reason, "bad signature");
            }
            other => panic!("expected EnvelopeRejected, got {other:?}"),
        }
        match &calls.events[1] {
            VideoEvent::EnvelopeRejected {
                community_id,
                sender_pseudonym,
                reason,
            } => {
                assert_eq!(community_id, "<unknown>");
                assert_eq!(sender_pseudonym, "<unknown>");
                assert_eq!(reason, "deserialize failed");
            }
            other => panic!("expected EnvelopeRejected sentinel, got {other:?}"),
        }
    }
}
