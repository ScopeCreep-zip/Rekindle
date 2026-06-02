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

use std::collections::HashMap;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::ipc::Channel;

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
    pub timestamp: u32,
    pub encoded_payload_b64: String,
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
        self.community.lock().insert(community_id, channel);
    }

    pub fn unregister_community(&self, community_id: &str) {
        self.community.lock().remove(community_id);
    }

    /// Push a frame to the community's channel. Same ephemeral + stale
    /// semantics as [`Self::send_dm`].
    pub fn send_community(&self, community_id: &str, frame: CommunityVideoFrameMsg) {
        let mut map = self.community.lock();
        let dead = map
            .get(community_id)
            .is_some_and(|ch| ch.send(frame).is_err());
        if dead {
            map.remove(community_id);
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
        assert_eq!(v["payloadB64"], "YmFy");
    }
}
