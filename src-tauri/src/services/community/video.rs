//! Architecture §10.6 video / screen-share orchestration.
//!
//! This module owns the per-community routing of `VideoFragment` /
//! `VideoParityFragment` / `FrameAck` / `KeyframeRequest` /
//! `BandwidthEstimate` / `MediaCapabilities` payloads. VP9 encode/decode
//! lives in the webview (browser `VideoEncoder`/`VideoDecoder` from
//! the WebCodecs API) — no system libvpx required. The desktop side
//! handles the MEK envelope + 28 KB fragmentation + Reed-Solomon FEC
//! parity (for keyframes) + gossip dispatch.
//!
//! **Camera vs screen share:** architecture §10.6 line 4083 calls for
//! "screen sharing: same pipeline, capture from OS screen capture API."
//! Capture lives in the webview alongside encode — `getUserMedia()` for
//! the camera and `getDisplayMedia()` for the screen, each producing a
//! `MediaStreamTrack` that feeds its own `VideoEncoder`. The two
//! streams are disambiguated on the wire by `derive_stream_id` taking
//! a `track_label` (`"camera"` / `"screen"`) so a single member can
//! ship both concurrently in the same channel without `stream_id`
//! collisions.
//!
//! Send pipeline (per architecture §10.6 line 2057):
//! `webview getUserMedia/getDisplayMedia` → `webview WebCodecs encode`
//! → `send_video_frame(...)` → `MEK encrypt` → `fragment_frame(...)`
//! (+ optional Reed-Solomon parity for keyframes) → for each shard emit
//! `ControlPayload::VideoFragment` / `VideoParityFragment` to the mesh.
//!
//! Receive pipeline: inbound `VideoFragment` and `VideoParityFragment`
//! payloads feed `Reassembler`, which surfaces complete frames via
//! `emit_video_frame` for the webview decoder. Frames recoverable only
//! via FEC are flagged so the UI can throttle the keyframe-request
//! cadence.

use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_video::Reassembler;

use crate::state::AppState;

/// Per-community reassembly state plus a set of stream_ids the local
/// sender has already announced via `TopologyChange`. Lives on
/// `AppState` so multiple active video streams across communities
/// don't collide.
#[derive(Default)]
pub struct VideoReassemblyState {
    inner: Mutex<std::collections::HashMap<String, Reassembler>>,
    /// `(community_id, stream_id)` pairs we've broadcast a
    /// `TopologyChange { reason: "initial" }` for. Cleared on logout
    /// via `clear()`. Bounded growth: one entry per concurrent
    /// outbound video stream — typically 1 or 2 per community.
    started_streams:
        Mutex<std::collections::HashSet<(String, [u8; 16])>>,
    /// Architecture §10.6 + §22 — per-stream Lamport clock of the
    /// last accepted `TopologyChange`. When two peers simultaneously
    /// switch the relay (mesh ↔ SFU), only the higher-lamport entry
    /// wins; lower-lamport messages are dropped so the reassembler
    /// doesn't flap between relays. Keyed by `(community_id, stream_id)`.
    last_topology_lamport:
        Mutex<std::collections::HashMap<(String, [u8; 16]), u64>>,
}

impl VideoReassemblyState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return true if this is the first call for `(community_id,
    /// stream_id)` since startup — used by `send_video_frame` to emit
    /// exactly one `TopologyChange { reason: "initial" }` per stream.
    pub fn mark_stream_started(&self, community_id: &str, stream_id: [u8; 16]) -> bool {
        let mut set = self.started_streams.lock();
        set.insert((community_id.to_string(), stream_id))
    }

    /// Ingest a data fragment for `community_id` from `sender_hex` and
    /// return the reassembled frame when complete. Errors map to
    /// `tracing::warn!` at the caller — they're recoverable; we just
    /// drop malformed fragments.
    pub fn ingest(
        &self,
        community_id: &str,
        sender_hex: &str,
        fragment: rekindle_video::VideoFragment,
        now_ms: u32,
    ) -> Option<rekindle_video::ReassembledFrame> {
        let mut map = self.inner.lock();
        let reassembler = map.entry(community_id.to_string()).or_default();
        match reassembler.ingest(sender_hex, fragment, now_ms) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "video fragment dropped");
                None
            }
        }
    }

    /// Ingest an FEC parity fragment. Returns `Some(frame)` if the
    /// parity completes the frame's reconstruction (combined with
    /// already-received data fragments).
    pub fn ingest_parity(
        &self,
        community_id: &str,
        sender_hex: &str,
        fragment: rekindle_video::VideoParityFragment,
        now_ms: u32,
    ) -> Option<rekindle_video::ReassembledFrame> {
        let mut map = self.inner.lock();
        let reassembler = map.entry(community_id.to_string()).or_default();
        match reassembler.ingest_parity(sender_hex, fragment, now_ms) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "video parity fragment dropped");
                None
            }
        }
    }

    /// Drop pending fragments for a stream — invoked when the local
    /// receiver sends a `KeyframeRequest`.
    pub fn reset_stream(
        &self,
        community_id: &str,
        stream_id: [u8; 16],
        sender_hex: &str,
    ) {
        let mut map = self.inner.lock();
        if let Some(reassembler) = map.get_mut(community_id) {
            reassembler.reset_stream(stream_id, sender_hex);
        }
    }

    /// Forget a community's reassembly state on logout / leave.
    pub fn forget(&self, community_id: &str) {
        self.inner.lock().remove(community_id);
        self.started_streams
            .lock()
            .retain(|(cid, _)| cid != community_id);
        self.last_topology_lamport
            .lock()
            .retain(|(cid, _), _| cid != community_id);
    }

    pub fn clear(&self) {
        self.inner.lock().clear();
        self.started_streams.lock().clear();
        self.last_topology_lamport.lock().clear();
    }

    /// Architecture §10.6 + §22 tie-break — return `true` when the
    /// incoming `TopologyChange` lamport is strictly greater than the
    /// last-seen lamport for `(community, stream)`. Updates the stored
    /// lamport on accept; rejects (returns false) for stale or
    /// duplicate-lamport messages, eliminating the flap that occurs
    /// when two peers race a relay handover.
    pub fn accept_topology_change(
        &self,
        community_id: &str,
        stream_id: [u8; 16],
        lamport: u64,
    ) -> bool {
        let key = (community_id.to_string(), stream_id);
        let mut map = self.last_topology_lamport.lock();
        let entry = map.entry(key).or_insert(0);
        if lamport > *entry {
            *entry = lamport;
            true
        } else {
            false
        }
    }
}

/// Compute the deterministic stream_id for
/// `(channel_id, sender_pseudonym, track_label)` per architecture §10.6
/// line 2063. Two members streaming in the same channel get distinct
/// stream_ids automatically (different `sender_pseudonym`); the same
/// member streaming twice (e.g. camera + screen share at once) passes
/// distinct `track_label`s — typically `"camera"` and `"screen"`. The
/// label is hashed in alongside the other inputs so it can be any UTF-8
/// string the caller picks.
pub fn derive_stream_id(
    channel_id: &str,
    sender_pseudonym_hex: &str,
    track_label: &str,
) -> [u8; 16] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(channel_id.as_bytes());
    hasher.update(b"|");
    hasher.update(sender_pseudonym_hex.as_bytes());
    hasher.update(b"|");
    hasher.update(track_label.as_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // The canonical labels live in the TypeScript `VideoTrackLabel`
    // union (frontend) and in the spec text — Rust-side they're just
    // arbitrary `&str` so a member can ship `"camera"`, `"screen"`,
    // or any custom label without the protocol caring.
    const CAMERA: &str = "camera";
    const SCREEN: &str = "screen";

    #[test]
    fn camera_and_screen_share_get_distinct_stream_ids() {
        let camera = derive_stream_id("ch1", "alice", CAMERA);
        let screen = derive_stream_id("ch1", "alice", SCREEN);
        assert_ne!(camera, screen);
    }

    #[test]
    fn same_label_is_deterministic_across_calls() {
        let a = derive_stream_id("ch1", "alice", CAMERA);
        let b = derive_stream_id("ch1", "alice", CAMERA);
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_senders_get_distinct_stream_ids() {
        let alice = derive_stream_id("ch1", "alice", CAMERA);
        let bob = derive_stream_id("ch1", "bob", CAMERA);
        assert_ne!(alice, bob);
    }
}

/// Forward a reassembled video frame to the frontend for decoding.
/// The actual VP9 decode + render pipeline lives in the webview side
/// (HTMLMediaElement + WebCodecs); the desktop side only ships the
/// MEK-decrypted bytes once they're complete.
pub fn emit_video_frame(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_hex: &str,
    frame: &rekindle_video::ReassembledFrame,
) {
    use tauri::Emitter as _;
    // MEK-decrypt the assembled payload before handing it to the UI.
    let mek_bytes = {
        let cache = state.mek_cache.lock();
        cache.get(community_id).map(|mek| (*mek.as_bytes(), mek.generation()))
    };
    let plaintext = if let Some((bytes, gen)) = mek_bytes {
        let mek = rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(bytes, gen);
        match mek.decrypt(&frame.payload) {
            Ok(plain) => plain,
            Err(e) => {
                tracing::warn!(error = %e, "video frame MEK decrypt failed");
                return;
            }
        }
    } else {
        tracing::debug!("video frame received but no MEK cached — dropping");
        return;
    };

    let _ = app_handle.emit(
        "community-event",
        crate::channels::CommunityEvent::VideoFrame {
            community_id: community_id.to_string(),
            sender_pseudonym: sender_hex.to_string(),
            stream_id: hex::encode(frame.stream_id),
            frame_seq: frame.frame_seq,
            keyframe: frame.keyframe,
            timestamp: frame.timestamp,
            payload_b64: base64_payload(&plaintext),
        },
    );
}

fn base64_payload(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Send-side entry point — invoked from the `send_video_frame` Tauri
/// command after the webview encoder produces a `VideoEncoder.encode()`
/// chunk. MEK-encrypts the payload, fragments to ≤28 KB, signs each
/// fragment with the sender's pseudonym Ed25519 key, and dispatches
/// each fragment as a `ControlPayload::VideoFragment` to the community
/// mesh.
///
/// One parity per N data shards for keyframes. With 4× ratio, dropping
/// up to 25% of fragments is recoverable. Inter-frames get no parity.
const KEYFRAME_PARITY_RATIO_DENOM: usize = 4;

/// Per-frame send request. Bundling all the variable-per-frame fields
/// into a struct keeps the orchestration helpers below a sane argument
/// count and gives the Tauri command a clear input shape.
#[derive(Debug)]
pub struct VideoFrameSend {
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub keyframe: bool,
    pub timestamp: u32,
    pub encoded_payload: Vec<u8>,
}

/// Send-side entry point. The frame's `encoded_payload` is the raw
/// VP9 bitstream chunk from WebCodecs — already compressed, not yet
/// MEK-encrypted. The webview maintains the monotonic `frame_seq`
/// counter per stream.
///
/// Keyframes ship with FEC (architecture §10.6 line 4080): one parity
/// shard per `KEYFRAME_PARITY_RATIO_DENOM` data shards (rounded up).
/// Inter-frames go without FEC — losing one is just a visible glitch
/// the next keyframe corrects.
pub fn send_video_frame(
    state: &crate::state::SharedState,
    community_id: &str,
    channel_id: &str,
    request: &VideoFrameSend,
) -> Result<u32, String> {
    if request.encoded_payload.is_empty() {
        return Err("empty encoded payload".to_string());
    }

    let (mek_bytes, mek_gen) = {
        let cache = state.mek_cache.lock();
        cache
            .get(community_id)
            .map(|m| (*m.as_bytes(), m.generation()))
            .ok_or_else(|| "no MEK cached for community — join voice/video first".to_string())?
    };
    let mek = rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(mek_bytes, mek_gen);
    let ciphertext = mek
        .encrypt(&request.encoded_payload)
        .map_err(|e| format!("MEK encrypt failed: {e}"))?;

    let signing_key = {
        let secret_opt: Option<[u8; 32]> = *state.identity_secret.lock();
        let secret = secret_opt.ok_or("identity not unlocked")?;
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id)
    };

    let ctx = SendCtx {
        state,
        community_id,
        channel_id,
        stream_id: request.stream_id,
        frame_seq: request.frame_seq,
        keyframe: request.keyframe,
        timestamp: request.timestamp,
        signing_key: &signing_key,
    };

    // Architecture §32 Phase 6 Week 22 — emit a `TopologyChange { reason
    // = "initial" }` exactly once per stream so receivers know to spin
    // up a decoder. The set of started streams is reset on logout via
    // `clear()`, so the very first `send_video_frame` per stream after
    // login fires the announcement; subsequent frames skip it.
    if state
        .video_reassembly
        .mark_stream_started(community_id, request.stream_id)
    {
        emit_initial_topology(state, community_id, channel_id, request.stream_id)?;
    }

    let parity_count = parity_count_for(request.keyframe, &ciphertext);
    if parity_count > 0 {
        ctx.send_with_fec(&ciphertext, parity_count)
    } else {
        ctx.send_without_fec(&ciphertext)
    }
}

fn emit_initial_topology(
    state: &crate::state::SharedState,
    community_id: &str,
    channel_id: &str,
    stream_id: [u8; 16],
) -> Result<(), String> {
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
    let lamport = crate::state_helpers::increment_lamport(state, community_id);
    let envelope = CommunityEnvelope::Control(ControlPayload::TopologyChange {
        channel_id: channel_id.to_string(),
        stream_id,
        // Initial topology is full-mesh broadcast — no relay host yet.
        // Future: when relay-peer routing lands for video, the elected
        // relay's pseudonym goes here and senders adjust their dispatch.
        relay_host_pseudonym: None,
        reason: "initial".to_string(),
        lamport,
    });
    crate::services::community::send_to_mesh(state, community_id, &envelope)
}

fn parity_count_for(keyframe: bool, ciphertext: &[u8]) -> u8 {
    if !keyframe {
        return 0;
    }
    let data = ciphertext.len().div_ceil(rekindle_video::FRAGMENT_PAYLOAD_LIMIT);
    if data < 2 {
        // 1-shard frames don't benefit from parity (parity = duplicate)
        // — and reed-solomon over 1+1 only recovers exact duplicates.
        return 0;
    }
    u8::try_from(data.div_ceil(KEYFRAME_PARITY_RATIO_DENOM)).unwrap_or(u8::MAX)
}

/// Bundle of references the FEC and non-FEC dispatch helpers both
/// need. Keeps each helper at one parameter (`ciphertext`) plus the
/// shared context.
struct SendCtx<'a> {
    state: &'a crate::state::SharedState,
    community_id: &'a str,
    channel_id: &'a str,
    stream_id: [u8; 16],
    frame_seq: u32,
    keyframe: bool,
    timestamp: u32,
    signing_key: &'a rekindle_secrets::ed25519_dalek::SigningKey,
}

impl SendCtx<'_> {
    fn send_without_fec(&self, ciphertext: &[u8]) -> Result<u32, String> {
        use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
        use rekindle_secrets::ed25519_dalek::Signer as _;

        let mut fragments = rekindle_video::fragment_frame(
            self.stream_id,
            self.frame_seq,
            self.keyframe,
            self.timestamp,
            ciphertext,
        )
        .map_err(|e| format!("fragment_frame: {e}"))?;
        let count = u32::try_from(fragments.len()).unwrap_or(u32::MAX);
        for fragment in &mut fragments {
            let to_sign = rekindle_video::fragment_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }
        for fragment in fragments {
            let envelope = CommunityEnvelope::Control(ControlPayload::VideoFragment {
                channel_id: self.channel_id.to_string(),
                stream_id: fragment.stream_id,
                frame_seq: fragment.frame_seq,
                frag_index: fragment.frag_index,
                frag_total: fragment.frag_total,
                keyframe: fragment.keyframe,
                timestamp: fragment.timestamp,
                payload: fragment.payload,
                signature: fragment.signature,
            });
            crate::services::community::send_to_mesh(self.state, self.community_id, &envelope)?;
        }
        Ok(count)
    }

    fn send_with_fec(&self, ciphertext: &[u8], parity_count: u8) -> Result<u32, String> {
        use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
        use rekindle_secrets::ed25519_dalek::Signer as _;

        let mut fec = rekindle_video::fragment_frame_with_fec(
            self.stream_id,
            self.frame_seq,
            self.keyframe,
            self.timestamp,
            ciphertext,
            parity_count,
        )
        .map_err(|e| format!("fragment_frame_with_fec: {e}"))?;

        for fragment in &mut fec.data {
            let to_sign = rekindle_video::fragment_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }
        for fragment in &mut fec.parity {
            let to_sign = rekindle_video::parity_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }

        let total = u32::try_from(fec.data.len() + fec.parity.len()).unwrap_or(u32::MAX);
        for fragment in fec.data {
            let envelope = CommunityEnvelope::Control(ControlPayload::VideoFragment {
                channel_id: self.channel_id.to_string(),
                stream_id: fragment.stream_id,
                frame_seq: fragment.frame_seq,
                frag_index: fragment.frag_index,
                frag_total: fragment.frag_total,
                keyframe: fragment.keyframe,
                timestamp: fragment.timestamp,
                payload: fragment.payload,
                signature: fragment.signature,
            });
            crate::services::community::send_to_mesh(self.state, self.community_id, &envelope)?;
        }
        for fragment in fec.parity {
            let envelope = CommunityEnvelope::Control(ControlPayload::VideoParityFragment {
                channel_id: self.channel_id.to_string(),
                stream_id: fragment.stream_id,
                frame_seq: fragment.frame_seq,
                parity_index: fragment.parity_index,
                parity_total: fragment.parity_total,
                data_count: fragment.data_count,
                frame_len: fragment.frame_len,
                timestamp: fragment.timestamp,
                payload: fragment.payload,
                signature: fragment.signature,
            });
            crate::services::community::send_to_mesh(self.state, self.community_id, &envelope)?;
        }
        Ok(total)
    }
}

/// Dispatch entry point — routed from `services/veilid/control_moderation`
/// when any video-flavoured `ControlPayload` arrives. Each variant flows
/// to its specific handler; the framing crate (`rekindle-video`) holds
/// the per-stream reassembly state and forwards complete frames to the
/// frontend via `emit_video_frame`.
pub fn handle_video_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use rekindle_protocol::dht::community::envelope::ControlPayload;
    use tauri::Emitter as _;

    match payload {
        ControlPayload::VideoFragment {
            channel_id: _,
            stream_id,
            frame_seq,
            frag_index,
            frag_total,
            keyframe,
            timestamp,
            payload,
            signature,
        } => {
            let frag = rekindle_video::VideoFragment {
                stream_id,
                frame_seq,
                frag_index,
                frag_total,
                keyframe,
                timestamp,
                payload,
                signature,
            };
            let now_ms = u32::try_from(rekindle_utils::timestamp_ms() % u64::from(u32::MAX))
                .unwrap_or(0);
            if let Some(frame) =
                state
                    .video_reassembly
                    .ingest(community_id, sender_pseudonym, frag, now_ms)
            {
                emit_video_frame(app_handle, state, community_id, sender_pseudonym, &frame);
            }
        }
        ControlPayload::VideoParityFragment {
            channel_id: _,
            stream_id,
            frame_seq,
            parity_index,
            parity_total,
            data_count,
            frame_len,
            timestamp,
            payload,
            signature,
        } => {
            let frag = rekindle_video::VideoParityFragment {
                stream_id,
                frame_seq,
                parity_index,
                parity_total,
                data_count,
                frame_len,
                timestamp,
                payload,
                signature,
            };
            let now_ms = u32::try_from(rekindle_utils::timestamp_ms() % u64::from(u32::MAX))
                .unwrap_or(0);
            if let Some(frame) =
                state
                    .video_reassembly
                    .ingest_parity(community_id, sender_pseudonym, frag, now_ms)
            {
                emit_video_frame(app_handle, state, community_id, sender_pseudonym, &frame);
            }
        }
        ControlPayload::FrameAck {
            channel_id,
            stream_id,
            last_frame_seq,
            kbps,
            loss_q8,
        } => {
            let _ = app_handle.emit(
                "community-event",
                crate::channels::CommunityEvent::VideoFrameAck {
                    community_id: community_id.to_string(),
                    sender_pseudonym: sender_pseudonym.to_string(),
                    channel_id,
                    stream_id: hex::encode(stream_id),
                    last_frame_seq,
                    kbps,
                    loss_q8,
                },
            );
        }
        ControlPayload::KeyframeRequest {
            channel_id,
            stream_id,
        } => {
            // Receiver dropped too many fragments; ask the frontend
            // (which owns the encoder) to mark the next frame as a
            // keyframe. Also reset our local reassembly buffer for
            // the same stream so we don't sit on stale partials.
            state
                .video_reassembly
                .reset_stream(community_id, stream_id, sender_pseudonym);
            let _ = app_handle.emit(
                "community-event",
                crate::channels::CommunityEvent::VideoKeyframeRequest {
                    community_id: community_id.to_string(),
                    sender_pseudonym: sender_pseudonym.to_string(),
                    channel_id,
                    stream_id: hex::encode(stream_id),
                },
            );
        }
        ControlPayload::BandwidthEstimate {
            channel_id,
            kbps,
            window_secs,
            loss_q8,
        } => {
            let _ = app_handle.emit(
                "community-event",
                crate::channels::CommunityEvent::VideoBandwidthEstimate {
                    community_id: community_id.to_string(),
                    sender_pseudonym: sender_pseudonym.to_string(),
                    channel_id,
                    kbps,
                    window_secs,
                    loss_q8,
                },
            );
        }
        ControlPayload::TopologyChange {
            channel_id,
            stream_id,
            relay_host_pseudonym,
            reason,
            lamport,
        } => {
            // Architecture §10.6 + §22 — drop simultaneous topology
            // changes with stale lamport so the reassembler doesn't
            // flap between relays when two peers race a handover. The
            // higher lamport wins; equal lamport is also rejected
            // (the first observation already won).
            if !state
                .video_reassembly
                .accept_topology_change(community_id, stream_id, lamport)
            {
                tracing::debug!(
                    community = %community_id,
                    stream = %hex::encode(stream_id),
                    sender = %sender_pseudonym,
                    incoming_lamport = lamport,
                    "ignoring stale TopologyChange"
                );
                return;
            }
            // Discard any partial frames buffered against the previous
            // relay so the next frame from the new relay starts clean.
            state
                .video_reassembly
                .reset_stream(community_id, stream_id, sender_pseudonym);
            let _ = app_handle.emit(
                "community-event",
                crate::channels::CommunityEvent::VideoTopologyChange {
                    community_id: community_id.to_string(),
                    sender_pseudonym: sender_pseudonym.to_string(),
                    channel_id,
                    stream_id: hex::encode(stream_id),
                    relay_host_pseudonym,
                    reason,
                    lamport,
                },
            );
        }
        ControlPayload::MediaCapabilities {
            channel_id,
            max_pixel_count,
            max_fps,
            codecs,
        } => {
            let _ = app_handle.emit(
                "community-event",
                crate::channels::CommunityEvent::VideoMediaCapabilities {
                    community_id: community_id.to_string(),
                    sender_pseudonym: sender_pseudonym.to_string(),
                    channel_id,
                    max_pixel_count,
                    max_fps,
                    codecs,
                },
            );
        }
        _ => {}
    }
}
