use rekindle_ipc::{HostApi, MEDIA_LANE_CAPACITY};
use rekindle_types::media::{MediaScope, Track};
use rekindle_types::video::{Codec, MediaFrame, UplinkFrame};

use crate::host::direct_tests::started;

fn scope() -> MediaScope {
    MediaScope::DirectCall {
        peer: "b".repeat(64),
    }
}

fn up(ts: u64) -> UplinkFrame {
    UplinkFrame {
        scope: scope(),
        track: Track::Camera,
        keyframe: ts == 0,
        codec: Codec::Vp8,
        payload: vec![1, 2, 3],
        timestamp_ms: ts,
    }
}

fn down(seq: u32) -> MediaFrame {
    MediaFrame {
        stream_id: "ab".into(),
        sequence: seq,
        keyframe: false,
        codec: Codec::Vp8,
        payload: vec![9],
        timestamp_ms: u64::from(seq),
        scope: scope(),
        sender: "b".repeat(64),
    }
}

#[tokio::test]
async fn the_uplink_is_taken_once_and_drops_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let host = started(&dir).await;
    let total = u64::try_from(MEDIA_LANE_CAPACITY).unwrap() + 3;
    for ts in 0..total {
        host.media_up(up(ts));
    }
    let mut rx = host
        .take_media_uplink()
        .expect("the engine takes the uplink");
    assert!(host.take_media_uplink().is_none(), "one consumer");
    assert_eq!(rx.len(), MEDIA_LANE_CAPACITY);
    assert_eq!(
        rx.recv().await.unwrap().timestamp_ms,
        3,
        "the 3 oldest dropped"
    );
}

#[tokio::test]
async fn each_downlink_stream_gets_every_frame_and_a_dropped_one_is_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let host = started(&dir).await;
    let mut a = host.media_down();
    let mut b = host.media_down();
    let cap = u32::try_from(MEDIA_LANE_CAPACITY).unwrap();
    for seq in 0..cap + 3 {
        assert_eq!(host.publish_media_down(&down(seq)), 2);
        assert_eq!(a.try_recv(), Some(down(seq)));
    }
    assert_eq!(b.len(), MEDIA_LANE_CAPACITY);
    assert_eq!(b.try_recv(), Some(down(3)));
    drop(b);
    assert_eq!(
        host.publish_media_down(&down(cap + 3)),
        1,
        "the dropped stream is pruned"
    );
    assert_eq!(a.try_recv(), Some(down(cap + 3)));
}
