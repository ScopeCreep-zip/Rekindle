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
/// pubkey hex; community channels by community id. Keying community by
/// id alone matches the single-slot voice engine: only one community
/// video panel is ever open at a time.
#[derive(Default)]
pub struct VideoChannelRegistry {
    dm: Mutex<HashMap<String, Channel<DmVideoFrameMsg>>>,
    community: Mutex<HashMap<String, Channel<CommunityVideoFrameMsg>>>,
    /// Frames that arrived before `register_community` — drained into
    /// the channel, in order, the moment it registers (Phase 6).
    pending_community: Mutex<HashMap<String, PendingFrames>>,
    /// The active native-capture self-view channel. Single slot: only
    /// one native camera session runs at a time. No pre-registration
    /// buffer — the panel registers before it starts the native session,
    /// and a dropped preview still is harmless (the next is a full JPEG).
    native_preview: Mutex<Option<Channel<NativePreviewFrameMsg>>>,
}

impl VideoChannelRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_dm(&self, peer_pubkey: String, channel: Channel<DmVideoFrameMsg>) {
        self.dm.lock().insert(peer_pubkey, channel);
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
            .is_some_and(|ch| ch.send(frame).is_err());
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

    pub fn register_native_preview(&self, channel: Channel<NativePreviewFrameMsg>) {
        *self.native_preview.lock() = Some(channel);
    }

    pub fn unregister_native_preview(&self) {
        *self.native_preview.lock() = None;
    }

    /// Push a self-view JPEG to the registered preview channel. No-op
    /// when none is registered; a dead channel (hard reload) is dropped
    /// so it can't linger.
    pub fn send_native_preview(&self, frame: NativePreviewFrameMsg) {
        let mut slot = self.native_preview.lock();
        let dead = slot.as_ref().is_some_and(|ch| ch.send(frame).is_err());
        if dead {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tauri::ipc::{Channel, InvokeResponseBody};

    /// A channel whose handler records every emitted JSON body, so tests
    /// can assert what the registry forwarded without a Tauri runtime.
    fn recording_channel<T: Serialize>() -> (Channel<T>, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&log);
        let channel = Channel::new(move |body: InvokeResponseBody| {
            if let InvokeResponseBody::Json(s) = body {
                sink.lock().push(s);
            }
            Ok(())
        });
        (channel, log)
    }

    fn dm_frame(peer: &str, seq: u32) -> DmVideoFrameMsg {
        DmVideoFrameMsg {
            peer_pubkey: peer.to_string(),
            stream_id_hex: "ab12".into(),
            frame_seq: seq,
            keyframe: true,
            codec: "vp9".into(),
            timestamp: 4321,
            encoded_payload_b64: "Zm9v".into(),
        }
    }

    #[test]
    fn send_dm_routes_to_registered_peer_with_camelcase_fields() {
        let reg = VideoChannelRegistry::new();
        let (channel, log) = recording_channel::<DmVideoFrameMsg>();
        reg.register_dm("peerA".into(), channel);

        reg.send_dm("peerA", dm_frame("peerA", 7));

        let got = log.lock();
        assert_eq!(got.len(), 1, "one frame forwarded");
        let v: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
        assert_eq!(v["peerPubkey"], "peerA");
        assert_eq!(v["streamIdHex"], "ab12");
        assert_eq!(v["frameSeq"], 7);
        assert_eq!(v["keyframe"], true);
        assert_eq!(v["codec"], "vp9");
        assert_eq!(v["encodedPayloadB64"], "Zm9v");
    }

    #[test]
    fn send_dm_to_unregistered_peer_is_silent_noop() {
        let reg = VideoChannelRegistry::new();
        // No registration, no panic, nothing delivered.
        reg.send_dm("ghost", dm_frame("ghost", 1));
    }

    #[test]
    fn unregister_dm_stops_delivery() {
        let reg = VideoChannelRegistry::new();
        let (channel, log) = recording_channel::<DmVideoFrameMsg>();
        reg.register_dm("peerA".into(), channel);
        reg.unregister_dm("peerA");

        reg.send_dm("peerA", dm_frame("peerA", 2));
        assert!(log.lock().is_empty(), "no delivery after unregister");
    }

    #[test]
    fn dm_frames_route_only_to_their_own_peer() {
        let reg = VideoChannelRegistry::new();
        let (channel_a, log_a) = recording_channel::<DmVideoFrameMsg>();
        let (channel_b, log_b) = recording_channel::<DmVideoFrameMsg>();
        reg.register_dm("peerA".into(), channel_a);
        reg.register_dm("peerB".into(), channel_b);

        reg.send_dm("peerA", dm_frame("peerA", 1));

        assert_eq!(log_a.lock().len(), 1);
        assert!(log_b.lock().is_empty(), "peerB channel untouched");
    }

    fn community_frame(seq: u32) -> CommunityVideoFrameMsg {
        CommunityVideoFrameMsg {
            community_id: "comm1".into(),
            sender_pseudonym: "psd".into(),
            stream_id: "ff00".into(),
            frame_seq: seq,
            keyframe: seq == 1,
            codec: "vp9".into(),
            timestamp: 0,
            payload_b64: "YmFy".into(),
        }
    }

    /// Phase 6 — frames that arrive before the panel registers are
    /// buffered and drained IN ORDER at registration.
    #[test]
    fn pre_registration_frames_drain_in_order() {
        let reg = VideoChannelRegistry::new();
        for seq in 1..=5 {
            reg.send_community("comm1", community_frame(seq));
        }
        let (channel, log) = recording_channel::<CommunityVideoFrameMsg>();
        reg.register_community("comm1".into(), channel);
        let got = log.lock();
        assert_eq!(got.len(), 5, "all buffered frames drained");
        let seqs: Vec<u64> = got
            .iter()
            .map(|s| {
                serde_json::from_str::<serde_json::Value>(s).unwrap()["frameSeq"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5], "drain preserves arrival order");
        drop(got);
        // Post-registration frames flow live.
        reg.send_community("comm1", community_frame(6));
        assert_eq!(log.lock().len(), 6);
    }

    /// Phase 6 — the pre-registration buffer is bounded: oldest evicted.
    #[test]
    fn pre_registration_buffer_evicts_oldest_at_cap() {
        let reg = VideoChannelRegistry::new();
        let overflow = 10;
        let total = u32::try_from(PENDING_COMMUNITY_FRAMES_MAX + overflow).unwrap();
        for seq in 1..=total {
            reg.send_community("comm1", community_frame(seq));
        }
        let (channel, log) = recording_channel::<CommunityVideoFrameMsg>();
        reg.register_community("comm1".into(), channel);
        let got = log.lock();
        assert_eq!(got.len(), PENDING_COMMUNITY_FRAMES_MAX);
        let first: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
        assert_eq!(
            first["frameSeq"].as_u64().unwrap(),
            u64::try_from(overflow).unwrap() + 1,
            "the oldest frames were the ones evicted"
        );
    }

    /// Phase 6 — unregister clears any pending buffer too.
    #[test]
    fn unregister_clears_pending_buffer() {
        let reg = VideoChannelRegistry::new();
        reg.send_community("comm1", community_frame(1));
        reg.unregister_community("comm1");
        let (channel, log) = recording_channel::<CommunityVideoFrameMsg>();
        reg.register_community("comm1".into(), channel);
        assert!(
            log.lock().is_empty(),
            "stale pending frames must not replay"
        );
    }

    #[test]
    fn native_preview_routes_then_stops_on_unregister() {
        let reg = VideoChannelRegistry::new();
        let (channel, log) = recording_channel::<NativePreviewFrameMsg>();
        reg.register_native_preview(channel);
        reg.send_native_preview(NativePreviewFrameMsg {
            stream_id_hex: "ab12".into(),
            jpeg_b64: "Zm9v".into(),
        });
        {
            let got = log.lock();
            assert_eq!(got.len(), 1, "one preview frame forwarded");
            let v: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
            assert_eq!(v["streamIdHex"], "ab12");
            assert_eq!(v["jpegB64"], "Zm9v");
        }
        reg.unregister_native_preview();
        reg.send_native_preview(NativePreviewFrameMsg {
            stream_id_hex: "ab12".into(),
            jpeg_b64: "YmFy".into(),
        });
        assert_eq!(log.lock().len(), 1, "no delivery after unregister");
    }

    #[test]
    fn send_community_routes_to_registered_community() {
        let reg = VideoChannelRegistry::new();
        let (channel, log) = recording_channel::<CommunityVideoFrameMsg>();
        reg.register_community("comm1".into(), channel);

        reg.send_community(
            "comm1",
            CommunityVideoFrameMsg {
                community_id: "comm1".into(),
                sender_pseudonym: "psd".into(),
                stream_id: "ff00".into(),
                frame_seq: 12,
                keyframe: false,
                codec: "vp9".into(),
                timestamp: 999,
                payload_b64: "YmFy".into(),
            },
        );

        let got = log.lock();
        assert_eq!(got.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
        assert_eq!(v["communityId"], "comm1");
        assert_eq!(v["senderPseudonym"], "psd");
        assert_eq!(v["frameSeq"], 12);
        assert_eq!(v["codec"], "vp9");
        assert_eq!(v["payloadB64"], "YmFy");
    }
}
