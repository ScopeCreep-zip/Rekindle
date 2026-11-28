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
    reg.register_dm("call-1", "peerA".into(), channel);

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
    reg.register_dm("call-1", "peerA".into(), channel);
    reg.unregister_dm("peerA");

    reg.send_dm("peerA", dm_frame("peerA", 2));
    assert!(log.lock().is_empty(), "no delivery after unregister");
}

#[test]
fn dm_frames_route_only_to_their_own_peer() {
    let reg = VideoChannelRegistry::new();
    let (channel_a, log_a) = recording_channel::<DmVideoFrameMsg>();
    let (channel_b, log_b) = recording_channel::<DmVideoFrameMsg>();
    reg.register_dm("call-1", "peerA".into(), channel_a);
    reg.register_dm("call-1", "peerB".into(), channel_b);

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
    reg.register_native_preview("call-1", channel);
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

/// Plan CL2: a destroyed call window's DM and self-view channels are
/// dropped; another window's stay.
#[test]
fn a_destroyed_windows_channels_are_dropped() {
    let reg = VideoChannelRegistry::new();
    let (gone, gone_log) = recording_channel::<DmVideoFrameMsg>();
    let (kept, kept_log) = recording_channel::<DmVideoFrameMsg>();
    let (preview, preview_log) = recording_channel::<NativePreviewFrameMsg>();
    reg.register_dm("call-1", "peerA".into(), gone);
    reg.register_dm("call-2", "peerB".into(), kept);
    reg.register_native_preview("call-1", preview);

    reg.release_window("call-1");

    reg.send_dm("peerA", dm_frame("peerA", 1));
    reg.send_dm("peerB", dm_frame("peerB", 1));
    reg.send_native_preview(NativePreviewFrameMsg {
        stream_id_hex: "ab12".into(),
        jpeg_b64: "Zm9v".into(),
    });
    assert!(
        gone_log.lock().is_empty(),
        "the destroyed window's DM channel was dropped"
    );
    assert!(
        preview_log.lock().is_empty(),
        "its self-view channel was dropped"
    );
    assert_eq!(kept_log.lock().len(), 1, "another window's channel stays");
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
