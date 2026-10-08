//! A roster peer's media link: its route, its [`RouteController`] and the
//! task that sends what the pacer releases (plan E4.3.3).
//!
//! Producers (the voice send loop, the video frame path) only enqueue and
//! wake the driver, so a slow `app_message` to one peer never holds up
//! another peer or the encoder (str0m's I/O loop: `poll_output`, send,
//! repeat).
//!
//! The driver hands each released datagram to one of two send lanes
//! ([`SendLane`]): voice and video-plane control, or video and padding. Each
//! lane sends in order, one `app_message` at a time; the two run side by
//! side, so a video hand-off that blocks (call 3: 2 s on Pop) never holds
//! up voice. Veilid statements promise dispatch, not order, and nothing in
//! veilid-core serialises them per destination beyond its compiled-route
//! and socket locks (`evidence/e4-3-transport-on-veilid.md` §7), so
//! concurrent sends to one route are safe.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::allocation::{allocate, Allocator, RouteAllocation};
use super::egress::{RouteController, RouteStats, SendLane, VideoFrame};
use super::VoiceFrameSender;
use crate::error::VoiceError;

/// A failure streak is logged at its first failure and every this-many
/// after (≈ one second of voice), not per datagram.
const FAILURE_LOG_EVERY: u64 = 50;

/// How often the driver logs the route's state.
const STATS_EVERY: Duration = Duration::from_secs(5);

/// One roster peer's media link.
pub struct PeerLink {
    controller: Mutex<RouteController>,
    padding_key: super::roster::PaddingKey,
    route: Mutex<Vec<u8>>,
    wake: Notify,
    stop: CancellationToken,
    sent: AtomicU64,
    failed: AtomicU64,
    /// A video datagram is being handed to Veilid.
    video_in_flight: std::sync::atomic::AtomicBool,
    /// Hand-off times per lane since the last stats line.
    voice_times: Mutex<SendTimes>,
    video_times: Mutex<SendTimes>,
}

impl PeerLink {
    #[must_use]
    pub fn new(route_blob: &[u8], padding_key: super::roster::PaddingKey) -> Arc<Self> {
        Arc::new(Self {
            controller: Mutex::new(RouteController::new()),
            padding_key,
            route: Mutex::new(route_blob.to_vec()),
            wake: Notify::new(),
            stop: CancellationToken::new(),
            sent: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            video_in_flight: std::sync::atomic::AtomicBool::new(false),
            voice_times: Mutex::new(SendTimes::default()),
            video_times: Mutex::new(SendTimes::default()),
        })
    }

    /// The route the peer advertised.
    #[must_use]
    pub fn route(&self) -> Vec<u8> {
        self.route.lock().clone()
    }

    pub fn set_route(&self, route_blob: &[u8]) {
        route_blob.clone_into(&mut self.route.lock());
    }

    /// Queue a voice packet or a video-plane control envelope ahead of
    /// paced media.
    pub fn enqueue_unpaced(&self, tag: u8, payload: Arc<[u8]>, media_bytes: usize) {
        self.controller
            .lock()
            .enqueue_unpaced(tag, payload, media_bytes, Instant::now());
        self.wake.notify_one();
    }

    /// Queue a video frame; `false` if the route refused it (paused, or
    /// waiting for a keyframe).
    pub fn enqueue_video(&self, frame: &VideoFrame) -> bool {
        let queued = self.controller.lock().enqueue_video(frame, Instant::now());
        self.wake.notify_one();
        queued
    }

    /// Hand the peer's feedback about our media to the controller, divide
    /// the new estimate and tell the estimator how far to probe. Returns
    /// the split.
    /// The route's round trip from the voice receiver reports (plan E4.3 T2).
    pub fn set_rtt(&self, rtt: std::time::Duration) {
        self.controller.lock().set_rtt(rtt);
    }

    pub fn on_feedback(&self, feedback: &TransportFeedback) -> RouteAllocation {
        let now = Instant::now();
        let mut c = self.controller.lock();
        let estimate = c.on_feedback(feedback, now);
        let video_offered = c.video_offered(now);
        let split = allocate(estimate, c.media_share(), video_offered);
        c.set_desired_bitrate(rekindle_media_bwe::Bitrate::bps(split.desired_on_wire), now);
        drop(c);
        self.wake.notify_one();
        split
    }

    /// Datagrams sent and failed so far.
    #[must_use]
    pub fn send_counts(&self) -> (u64, u64) {
        (
            self.sent.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed),
        )
    }

    /// End the driver.
    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// Whether [`Self::stop`] was called.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stop.is_cancelled()
    }

    /// Run the driver until [`Self::stop`] or the scope's cancellation:
    /// the release loop and the two send lanes, side by side.
    pub async fn drive(
        self: Arc<Self>,
        peer: String,
        sender: Arc<dyn VoiceFrameSender>,
        allocator: Arc<Allocator>,
        scope_stop: CancellationToken,
    ) {
        let (voice_tx, voice_rx) = tokio::sync::mpsc::unbounded_channel();
        // The video lane takes one datagram at a time; the controller keeps
        // the rest while it is busy.
        let (video_tx, video_rx) = tokio::sync::mpsc::channel(1);
        tokio::join!(
            self.release(&peer, &allocator, &scope_stop, voice_tx, video_tx),
            self.lane(&peer, &*sender, SendLane::Voice, LaneRx::Voice(voice_rx)),
            self.lane(&peer, &*sender, SendLane::Video, LaneRx::Video(video_rx)),
        );
    }

    /// Release what the pacer lets go and hand it to its lane; never
    /// awaits a send. Ends (closing both lanes) on stop.
    async fn release(
        &self,
        peer: &str,
        allocator: &Allocator,
        scope_stop: &CancellationToken,
        voice_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
        video_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        let mut last_stats = Instant::now();
        loop {
            let now = Instant::now();
            let (polled, next, keyframe, stats) = {
                let mut c = self.controller.lock();
                c.set_video_lane_busy(self.video_in_flight.load(Ordering::Acquire));
                c.handle_timeout(now);
                let polled = c.poll_datagram(now);
                let stats = (now.duration_since(last_stats) >= STATS_EVERY).then(|| c.stats(now));
                (polled, c.poll_timeout(), c.take_keyframe_wanted(), stats)
            };
            if keyframe {
                allocator.request_keyframe();
            }
            if let Some(stats) = stats {
                last_stats = now;
                self.log_stats(peer, &stats);
            }
            if let Some((mut datagram, lane)) = polled {
                if datagram.first() == Some(&crate::media_frame::PADDING_TAG) {
                    let signed =
                        self.padding_key.read().as_ref().is_some_and(|key| {
                            crate::media_frame::sign_padding(&mut datagram, key)
                        });
                    if !signed {
                        // Unsigned padding would be dropped on arrival.
                        continue;
                    }
                }
                match lane {
                    SendLane::Voice => {
                        let _ = voice_tx.send(datagram);
                    }
                    SendLane::Video => {
                        self.video_in_flight.store(true, Ordering::Release);
                        if video_tx.try_send(datagram).is_err() {
                            // Not reachable: the controller hides video
                            // while a hand-off is in flight.
                            self.video_in_flight.store(false, Ordering::Release);
                        }
                    }
                }
                continue;
            }
            let sleep = async {
                match next {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                biased;
                () = self.stop.cancelled() => return,
                () = scope_stop.cancelled() => return,
                () = self.wake.notified() => {}
                () = sleep => {}
            }
        }
    }

    /// One send lane: hand each datagram to Veilid in order, until the
    /// release loop ends.
    async fn lane(
        &self,
        peer: &str,
        sender: &dyn VoiceFrameSender,
        lane: SendLane,
        mut rx: LaneRx,
    ) {
        let mut streak = 0u64;
        while let Some(datagram) = rx.recv().await {
            let route = self.route();
            let seq = crate::media_frame::split_sequenced(&datagram).map(|(_, seq, _)| seq);
            let started = Instant::now();
            let result = sender.send_voice_frame(&route, datagram).await;
            let times = match lane {
                SendLane::Voice => &self.voice_times,
                SendLane::Video => &self.video_times,
            };
            times.lock().note(started.elapsed());
            match result {
                Ok(()) => {
                    if let Some(seq) = seq {
                        self.controller.lock().on_handed_off(seq, Instant::now());
                    }
                    self.sent.fetch_add(1, Ordering::Relaxed);
                    streak = 0;
                }
                Err(e) => {
                    self.failed.fetch_add(1, Ordering::Relaxed);
                    streak += 1;
                    log_failure(peer, streak, &e);
                }
            }
            if lane == SendLane::Video {
                self.video_in_flight.store(false, Ordering::Release);
                self.wake.notify_one();
            }
        }
    }

    fn log_stats(&self, peer: &str, s: &RouteStats) {
        let (sent, failed) = self.send_counts();
        let (voice_send_p50_ms, voice_send_p95_ms, voice_send_max_ms) =
            self.voice_times.lock().take();
        let (send_p50_ms, send_p95_ms, send_max_ms) = self.video_times.lock().take();
        tracing::info!(
            peer = %peer,
            voice_send_p50_ms,
            voice_send_p95_ms,
            voice_send_max_ms,
            send_p50_ms,
            send_p95_ms,
            send_max_ms,
            estimate_kbps = s.estimate_bps / 1_000,
            overusing = s.overusing,
            video_queue_ms = s.video_queue_ms,
            dropped_video_frames = s.dropped_video,
            feedback_reports = s.feedback_reports,
            reported_received = s.reported_received,
            reported_lost = s.reported_lost,
            acked_kbps = s.bwe.acked.map(|b| b.as_u64() / 1_000),
            delay_kbps = s.bwe.delay.map(|b| b.as_u64() / 1_000),
            loss_kbps = s.bwe.loss.map(|b| b.as_u64() / 1_000),
            loss_state = s.bwe.loss_state,
            backoff_cuts = s.bwe.backoff_cuts,
            probes_applied = s.bwe.probes_applied,
            datagrams = s.sent,
            sent,
            failed,
            "media route"
        );
    }
}

/// Send hand-off times over one stats window, in microseconds.
#[derive(Default)]
struct SendTimes(Vec<u64>);

impl SendTimes {
    fn note(&mut self, took: Duration) {
        self.0
            .push(u64::try_from(took.as_micros()).unwrap_or(u64::MAX));
    }

    /// Median, 95th percentile and maximum, ms, and start a new window.
    fn take(&mut self) -> (f64, f64, f64) {
        let mut v = std::mem::take(&mut self.0);
        if v.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        v.sort_unstable();
        let at = |pct: usize| {
            let rank = (v.len() * pct).div_ceil(100).max(1) - 1;
            ms(v[rank.min(v.len() - 1)])
        };
        (at(50), at(95), ms(v[v.len() - 1]))
    }
}

fn ms(us: u64) -> f64 {
    f64::from(u32::try_from(us).unwrap_or(u32::MAX)) / 1_000.0
}

fn log_failure(peer: &str, streak: u64, error: &VoiceError) {
    if streak == 1 || streak.is_multiple_of(FAILURE_LOG_EVERY) {
        // A route that keeps returning `NoConnection` is one-way dead: our
        // datagrams cannot reach the peer.
        tracing::warn!(peer = %peer, consecutive_failures = streak, error = %error,
            one_way_dead = super::is_no_connection(error), "media send failing for peer");
    }
}

/// A lane's receiving end: voice is unbounded (never held), video takes
/// one datagram at a time.
enum LaneRx {
    Voice(tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>),
    Video(tokio::sync::mpsc::Receiver<Vec<u8>>),
}

impl LaneRx {
    async fn recv(&mut self) -> Option<Vec<u8>> {
        match self {
            Self::Voice(rx) => rx.recv().await,
            Self::Video(rx) => rx.recv().await,
        }
    }
}
