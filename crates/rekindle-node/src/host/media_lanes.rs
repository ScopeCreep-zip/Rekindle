//! The Host's in-process encoded-media lanes (plan E4.1): the queues behind
//! `HostApi::media_up` and `HostApi::media_down`. Every hop is bounded and
//! drop-oldest (`rekindle_ipc::media_channel`), because a late video frame
//! is worthless. In-process frontends reach them directly; F1's
//! `DaemonClient` maps them onto `BusPayload::MediaUp`/`Media`.

use parking_lot::Mutex;
use rekindle_ipc::{media_channel, MediaReceiver, MediaSender, MEDIA_LANE_CAPACITY};
use rekindle_types::video::{MediaFrame, UplinkFrame};

/// One uplink (every frontend into the video engine) and one downlink
/// stream per `media_down` caller.
pub struct MediaLanes {
    up: MediaSender<UplinkFrame>,
    /// The uplink's consumer, until the video engine takes it.
    up_rx: Mutex<Option<MediaReceiver<UplinkFrame>>>,
    /// One sender per open downlink stream; a dropped stream is pruned.
    down: Mutex<Vec<MediaSender<MediaFrame>>>,
}

impl MediaLanes {
    #[must_use]
    pub fn new() -> Self {
        let (up, up_rx) = media_channel(MEDIA_LANE_CAPACITY);
        Self {
            up,
            up_rx: Mutex::new(Some(up_rx)),
            down: Mutex::new(Vec::new()),
        }
    }

    /// Queue an uplink frame; the oldest is dropped when the lane is full.
    pub fn push_up(&self, frame: UplinkFrame) {
        if self.up.send(frame) {
            tracing::trace!("media uplink lane full, dropped oldest frame");
        }
    }

    /// The uplink's consumer: `Some` once.
    pub fn take_uplink(&self) -> Option<MediaReceiver<UplinkFrame>> {
        self.up_rx.lock().take()
    }

    /// Open a downlink stream.
    pub fn open_down(&self) -> MediaReceiver<MediaFrame> {
        let (tx, rx) = media_channel(MEDIA_LANE_CAPACITY);
        self.down.lock().push(tx);
        rx
    }

    /// Fan `frame` out to every open downlink stream, pruning the closed
    /// ones; returns how many streams it reached.
    pub fn push_down(&self, frame: &MediaFrame) -> usize {
        let mut down = self.down.lock();
        down.retain(|tx| !tx.is_closed());
        for tx in down.iter() {
            tx.send(frame.clone());
        }
        down.len()
    }
}

impl Default for MediaLanes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use rekindle_types::media::{MediaScope, Track};
    use rekindle_types::video::Codec;

    use super::*;

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
        let lanes = MediaLanes::new();
        let total = u64::try_from(MEDIA_LANE_CAPACITY).unwrap() + 3;
        for ts in 0..total {
            lanes.push_up(up(ts));
        }
        let mut rx = lanes.take_uplink().expect("the engine takes the uplink");
        assert!(lanes.take_uplink().is_none(), "one consumer");
        assert_eq!(rx.len(), MEDIA_LANE_CAPACITY);
        assert_eq!(
            rx.recv().await.unwrap().timestamp_ms,
            3,
            "the 3 oldest dropped"
        );
    }

    #[tokio::test]
    async fn each_downlink_stream_gets_every_frame_and_a_dropped_one_is_pruned() {
        let lanes = MediaLanes::new();
        let mut a = lanes.open_down();
        let b = lanes.open_down();
        assert_eq!(lanes.push_down(&down(1)), 2);
        drop(b);
        assert_eq!(lanes.push_down(&down(2)), 1, "the dropped stream is pruned");
        assert_eq!(a.recv().await.unwrap(), down(1));
        assert_eq!(a.recv().await.unwrap(), down(2));
    }
}
