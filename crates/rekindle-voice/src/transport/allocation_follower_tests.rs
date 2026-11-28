use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_types::media::Track;

use super::{
    encoder_kbps, Allocation, Allocator, VideoAllocationFollower, VideoEncoders, VIDEO_MIN_BPS,
};

#[derive(Default)]
struct Encoders {
    native_rates: Mutex<Vec<u32>>,
    webview_rates: Mutex<Vec<u32>>,
    keyframes: Mutex<Vec<Option<Track>>>,
}

impl VideoEncoders for Encoders {
    fn set_native_kbps(&self, kbps: u32) {
        self.native_rates.lock().push(kbps);
    }

    fn send_webview_kbps(&self, kbps: u32) {
        self.webview_rates.lock().push(kbps);
    }

    fn force_native_keyframes(&self) {
        self.keyframes.lock().push(None);
    }

    fn request_webview_keyframe(&self, track: Track) {
        self.keyframes.lock().push(Some(track));
    }
}

fn follower() -> VideoAllocationFollower<Encoders> {
    VideoAllocationFollower {
        encoders: Encoders::default(),
        allocator: Arc::new(Allocator::new()),
        community_id: String::new(),
        channel_id: String::new(),
    }
}

#[test]
fn the_webview_is_retargeted_only_past_the_hysteresis() {
    let follower = follower();
    let mut current = None;
    let estimates = [300, 330, 345, 350, 250];
    for kbps in estimates {
        let allocation = Allocation {
            video_bps: kbps * 1_000,
            ..Allocation::default()
        };
        follower.apply(allocation, &mut current);
    }
    assert_eq!(follower.encoders.native_rates.lock().as_slice(), estimates);
    assert_eq!(
        follower.encoders.webview_rates.lock().as_slice(),
        [300, 350, 250]
    );
}

#[test]
fn the_encoder_rate_never_drops_below_video_minimum() {
    let starved = Allocation {
        video_bps: 0,
        ..Allocation::default()
    };
    assert_eq!(encoder_kbps(starved), VIDEO_MIN_BPS / 1_000);
}

#[test]
fn a_keyframe_request_reaches_native_and_both_webview_tracks() {
    let follower = follower();
    follower.request_keyframe();
    assert_eq!(
        follower.encoders.keyframes.lock().as_slice(),
        [None, Some(Track::Camera), Some(Track::Screen)]
    );
}
