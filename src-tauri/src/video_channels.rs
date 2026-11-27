//! Phase 11 Tier 1 — per-stream `tauri::ipc::Channel` registry for the
//! two genuinely high-throughput Rust→frontend streams: DM video frames
//! and community video frames.
//!
//! Tauri's event system "is not designed for low latency or high
//! throughput situations"; channels are. Video frames arrive at the
//! camera/encoder cadence (~15-30 fps) carrying base64 VP9 payloads, so
//! they bypass the shared `event_dispatch` mpsc queue and go straight to
//! a per-stream channel the frontend registers at panel mount and drops
//! at panel unmount.
//!
//! Voice frames are deliberately NOT here: in this codebase audio never
//! crosses to the webview — capture/encode/decode/playback all live in
//! `rekindle-voice` (cpal). The `voice-event` channel carries only UI
//! signaling (join/leave/speaking/mute/quality) at a handful of events
//! per call, so it stays on the centralized `event_dispatch` bus.

use std::collections::{HashMap, VecDeque};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::ipc::Channel;

/// Phase 6 — bound on community frames buffered while the panel hasn't
/// registered its channel yet (remote senders can already be streaming
/// when we join: their frames physically arrive before mount). ~6 s of
/// one 15 fps stream; oldest evicted beyond it.
pub const PENDING_COMMUNITY_FRAMES_MAX: usize = 90;

/// Pre-registration frame buffer for one community.
#[derive(Default)]
struct PendingFrames {
    frames: VecDeque<CommunityVideoFrameMsg>,
    evicted: u64,
}

/// One reassembled DM video frame pushed to the per-peer channel the DM
/// video panel registers. Mirrors the previous `dm-video-frame` event
/// payload minus the now-redundant `kind` discriminator (the channel is
/// dedicated to frames).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmVideoFrameMsg {
    pub peer_pubkey: String,
    pub stream_id_hex: String,
    pub frame_seq: u32,
    pub keyframe: bool,
    /// Codec wire string — the receiver configures its decoder from
    /// this tag (RTP payload-type analog).
    pub codec: String,
    pub timestamp: u32,
    pub encoded_payload_b64: String,
}

/// One JPEG still from the Linux-native capture pipeline's preview
/// branch — the LOCAL self-view, pushed to the channel the video panel
/// registers while a native session runs. Codec-stateless: the webview
/// paints it straight to a canvas via `createImageBitmap`, no decoder.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePreviewFrameMsg {
    pub stream_id_hex: String,
    pub jpeg_b64: String,
}

/// One reassembled community video frame pushed to the per-community
/// channel the community video panel registers. Mirrors the previous
/// `CommunityEvent::VideoFrame` payload.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityVideoFrameMsg {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub stream_id: String,
    pub frame_seq: u32,
    pub keyframe: bool,
    /// Codec wire string — the receiver configures its decoder from
    /// this tag (RTP payload-type analog).
    pub codec: String,
    pub timestamp: u32,
    pub payload_b64: String,
}

/// Frontend-registered video channels. DM channels are keyed by peer
/// pubkey hex; community channels by community id. A DM channel and the
/// native self-view channel remember the window that registered them
/// (plan CL2): a call window's are dropped when it is destroyed
/// ([`Self::release_window`]), whether the call ended or it was closed. Keying community by
/// id alone matches the single-slot voice engine: only one community
/// video panel is ever open at a time.
#[derive(Default)]
pub struct VideoChannelRegistry {
    dm: Mutex<HashMap<String, (String, Channel<DmVideoFrameMsg>)>>,
    community: Mutex<HashMap<String, Channel<CommunityVideoFrameMsg>>>,
    /// Frames that arrived before `register_community` — drained into
    /// the channel, in order, the moment it registers (Phase 6).
    pending_community: Mutex<HashMap<String, PendingFrames>>,
    /// The active native-capture self-view channel. Single slot: only
    /// one native camera session runs at a time. No pre-registration
    /// buffer — the panel registers before it starts the native session,
    /// and a dropped preview still is harmless (the next is a full JPEG).
    native_preview: Mutex<Option<(String, Channel<NativePreviewFrameMsg>)>>,
}

impl VideoChannelRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `window`'s channel for `peer_pubkey`'s frames.
    pub fn register_dm(
        &self,
        window: &str,
        peer_pubkey: String,
        channel: Channel<DmVideoFrameMsg>,
    ) {
        self.dm
            .lock()
            .insert(peer_pubkey, (window.to_owned(), channel));
    }

    /// Drop every channel `window` registered (it was destroyed): its
    /// DM video channels and its native self-view channel. The webview
    /// that listened on them is gone, so nothing would read their frames.
    pub fn release_window(&self, window: &str) {
        self.dm.lock().retain(|_, (owner, _)| owner != window);
        let mut preview = self.native_preview.lock();
        if preview.as_ref().is_some_and(|(owner, _)| owner == window) {
            *preview = None;
        }
    }

    pub fn unregister_dm(&self, peer_pubkey: &str) {
        self.dm.lock().remove(peer_pubkey);
    }

    /// Push a frame to the peer's channel. No-op when the panel never
    /// registered — the frame is ephemeral. If the frontend went away
    /// without unregistering (hard reload), `send` errors and the stale
    /// channel is dropped so it can't accumulate.
    pub fn send_dm(&self, peer_pubkey: &str, frame: DmVideoFrameMsg) {
        let mut map = self.dm.lock();
        let dead = map
            .get(peer_pubkey)
            .is_some_and(|(_, ch)| ch.send(frame).is_err());
        if dead {
            map.remove(peer_pubkey);
        }
    }

    pub fn register_community(
        &self,
        community_id: String,
        channel: Channel<CommunityVideoFrameMsg>,
    ) {
        // Phase 6 — drain frames that beat the registration, in order.
        // Post-drain mid-GOP deltas self-heal via the panel's
        // 15-dropped-deltas keyframe-request escalation.
        let pending = self.pending_community.lock().remove(&community_id);
        if let Some(pending) = pending {
            let drained = pending.frames.len();
            for frame in pending.frames {
                if channel.send(frame).is_err() {
                    // Channel already dead (hard reload mid-drain):
                    // nothing to register.
                    return;
                }
            }
            tracing::info!(
                target: "rekindle_video::receive",
                community_id = %community_id,
                drained,
                evicted = pending.evicted,
                "flushed pre-registration video frames"
            );
        }
        self.community.lock().insert(community_id, channel);
    }

    pub fn unregister_community(&self, community_id: &str) {
        self.community.lock().remove(community_id);
        self.pending_community.lock().remove(community_id);
    }

    /// Push a frame to the community's channel. Stale-channel semantics
    /// as [`Self::send_dm`]; an UNREGISTERED community buffers the
    /// frame (bounded) instead of dropping it — the panel-mount /
    /// remote-sender race would otherwise eat the first seconds of
    /// video including the keyframe (Phase 6).
    pub fn send_community(&self, community_id: &str, frame: CommunityVideoFrameMsg) {
        let mut map = self.community.lock();
        if let Some(ch) = map.get(community_id) {
            if ch.send(frame).is_err() {
                map.remove(community_id);
            }
        } else {
            drop(map);
            let mut pending = self.pending_community.lock();
            let entry = pending.entry(community_id.to_string()).or_default();
            if entry.frames.len() >= PENDING_COMMUNITY_FRAMES_MAX {
                entry.frames.pop_front();
                entry.evicted += 1;
                if entry.evicted == 1 || entry.evicted.is_multiple_of(30) {
                    tracing::warn!(
                        target: "rekindle_video::receive",
                        community_id = %community_id,
                        evicted = entry.evicted,
                        "community video frames arriving before panel registration — buffering (evicting oldest)"
                    );
                }
            }
            entry.frames.push_back(frame);
        }
    }

    /// Register `window`'s self-view channel (one slot).
    pub fn register_native_preview(&self, window: &str, channel: Channel<NativePreviewFrameMsg>) {
        *self.native_preview.lock() = Some((window.to_owned(), channel));
    }

    pub fn unregister_native_preview(&self) {
        *self.native_preview.lock() = None;
    }

    /// Push a self-view JPEG to the registered preview channel. No-op
    /// when none is registered; a dead channel (hard reload) is dropped
    /// so it can't linger.
    pub fn send_native_preview(&self, frame: NativePreviewFrameMsg) {
        let mut slot = self.native_preview.lock();
        let dead = slot.as_ref().is_some_and(|(_, ch)| ch.send(frame).is_err());
        if dead {
            *slot = None;
        }
    }
}

#[cfg(test)]
#[path = "video_channels_tests.rs"]
mod tests;
