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
pub(super) struct MediaLanes {
    up: MediaSender<UplinkFrame>,
    /// The uplink's consumer, until the video engine takes it.
    up_rx: Mutex<Option<MediaReceiver<UplinkFrame>>>,
    /// One sender per open downlink stream; a dropped stream is pruned.
    down: Mutex<Vec<MediaSender<MediaFrame>>>,
}

impl MediaLanes {
    #[must_use]
    pub(super) fn new() -> Self {
        let (up, up_rx) = media_channel(MEDIA_LANE_CAPACITY);
        Self {
            up,
            up_rx: Mutex::new(Some(up_rx)),
            down: Mutex::new(Vec::new()),
        }
    }

    /// Queue an uplink frame; the oldest is dropped when the lane is full.
    pub(super) fn push_up(&self, frame: UplinkFrame) {
        if self.up.send(frame) {
            tracing::trace!("media uplink lane full, dropped oldest frame");
        }
    }

    /// The uplink's consumer: `Some` once.
    pub(super) fn take_uplink(&self) -> Option<MediaReceiver<UplinkFrame>> {
        self.up_rx.lock().take()
    }

    /// Open a downlink stream.
    pub(super) fn open_down(&self) -> MediaReceiver<MediaFrame> {
        let (tx, rx) = media_channel(MEDIA_LANE_CAPACITY);
        let mut down = self.down.lock();
        down.retain(|tx| !tx.is_closed());
        down.push(tx);
        rx
    }

    /// Fan `frame` out to every open downlink stream, pruning the closed
    /// ones; returns how many streams it reached.
    pub(super) fn push_down(&self, frame: &MediaFrame) -> usize {
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
#[path = "media_lanes_tests.rs"]
mod tests;
