//! In-memory state for an active or pending direct call.
//!
//! Held by the Tauri shell in `AppState.active_calls` (DashMap keyed by
//! `call_id`). The status flips through Outgoing → Active on `CallAccept`,
//! Outgoing → Missed on the 30 s ring timeout, etc.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use rekindle_secrets::sframe::SframeSender;
use x25519_dalek::StaticSecret;
use zeroize::Zeroizing;

/// Whether the call is audio-only or video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    Audio,
    Video,
}

impl CallKind {
    /// Wire encoding (matches `CallOffer.offer_kind`).
    #[must_use]
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Audio => 0,
            Self::Video => 1,
        }
    }

    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Audio),
            1 => Some(Self::Video),
            _ => None,
        }
    }
}

/// Lifecycle state machine for a direct call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallStatus {
    /// Local user dialed; awaiting `CallAccept` / `CallDecline` /
    /// timeout.
    Outgoing,
    /// Remote peer dialed; awaiting local accept/decline.
    Incoming,
    /// Wave 13 — accept arrived (caller side) or user clicked Accept
    /// (receiver side); voice transport is being brought up but not
    /// yet ready to carry frames. Brief intermediate state — exists
    /// so the UI can distinguish "we sent the accept envelope but
    /// audio isn't flowing yet" from "we're talking".
    Connecting,
    /// Both sides exchanged keys; voice transport is up.
    Active,
    /// Ring expired without an accept (architecture §10.10 — 30 s).
    Missed,
}

/// Per-call state tracked while the call is live.
///
/// `my_x25519_secret` is dropped (and zeroized) when the entry leaves
/// `AppState.active_calls`, preventing the secret from outliving the
/// call. `media_secret` is computed once on accept; the voice session
/// reads it, with `media_sender`, through its `MediaKeySource`.
#[derive(Clone)]
pub struct CallState {
    pub call_id: String,
    /// Hex-encoded Ed25519 identity key of the remote peer.
    pub peer_pubkey: String,
    pub kind: CallKind,
    pub status: CallStatus,
    /// Unix milliseconds when the ring expires. The Outgoing-side ring
    /// timer also writes a `missed_calls` row at this instant.
    pub expires_at_ms: u64,
    /// Local X25519 secret, kept until the call ends.
    pub my_x25519_secret: Option<StaticSecret>,
    /// Peer's X25519 public key, captured from `CallOffer` (Incoming
    /// side) or `CallAccept` (Outgoing side).
    pub peer_x25519_pub: Option<[u8; 32]>,
    /// The call's SFrame scope secret (X25519 ECDH + HKDF, see
    /// [`crate::derive_call_key`]). `None` until both sides have
    /// exchanged X25519 publics. Zeroized on drop.
    pub media_secret: Option<Zeroizing<[u8; 32]>>,
    /// Our SFrame sender state for this call (session tag + CTR). Shared
    /// by every clone of the state, so a rebuilt voice transport
    /// continues the counter (RFC 9605 §9.1).
    pub media_sender: Arc<SframeSender>,
    /// Phase 5 — codecs the PEER's WebView can decode
    /// (preference-ordered wire strings: "vp9" / "vp8" / "h264").
    /// Captured from `CallInvite` (Incoming side) or `CallAccept`
    /// (Outgoing side). Empty = peer predates its probe (or sent
    /// nothing) — senders treat that as the VP9 floor.
    pub peer_video_decode_codecs: Vec<String>,
}

impl std::fmt::Debug for CallState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallState")
            .field("call_id", &self.call_id)
            .field("peer_pubkey", &self.peer_pubkey)
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("has_secret", &self.my_x25519_secret.is_some())
            .field("has_peer_pub", &self.peer_x25519_pub.is_some())
            .field("has_media_secret", &self.media_secret.is_some())
            .field("media_sender", &self.media_sender)
            .field("peer_video_decode_codecs", &self.peer_video_decode_codecs)
            .finish()
    }
}
