//! A roster peer's media link: its route, its [`RouteController`] and the
//! task that sends what the pacer releases (plan E4.3.3).
//!
//! Producers (the voice send loop, the video frame path) only enqueue and
//! wake the driver; the driver alone talks to Veilid, one message at a
//! time, so a slow `app_message` to one peer never holds up another peer
//! or the encoder (str0m's I/O loop: `poll_output`, send, repeat).
//!
//! Each message carries everything due at that wake (plan E4.3 T3): the
//! driver drains the controller into one [`bundle`](crate::bundle), and
//! control datagrams (transport feedback, receiver reports) ride in it,
//! waiting at most [`CONTROL_ATTACH_WAIT`] for media to ride with. Every
//! Veilid message costs ~1,660 B whatever it carries, so the count of
//! messages, not their bytes, set a call's fixed demand (calls 1–3).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::allocation::{allocate, Allocator, RouteAllocation};
use super::egress::{RouteController, RouteStats, VideoFrame};
use super::VoiceFrameSender;
use crate::error::VoiceError;

/// A failure streak is logged at its first failure and every this-many
/// after (≈ one second of voice), not per datagram.
const FAILURE_LOG_EVERY: u64 = 50;

/// How often the driver logs the route's state.
const STATS_EVERY: Duration = Duration::from_secs(5);

/// Longest a control datagram waits for media to share a message with:
/// one Opus frame. Feedback and reports cost a whole message alone (DONAR
/// carries its measurement inside media for the same reason, NSDI 2022).
const CONTROL_ATTACH_WAIT: Duration = Duration::from_millis(20);

/// One roster peer's media link.
pub struct PeerLink {
    controller: Mutex<RouteController>,
    padding_key: super::roster::PaddingKey,
    route: Mutex<Vec<u8>>,
    wake: Notify,
    stop: CancellationToken,
    sent: AtomicU64,
    failed: AtomicU64,
    /// Messages handed to Veilid (a message may carry several datagrams).
    messages: AtomicU64,
    /// Control datagrams waiting to ride a media message, with when each
    /// was queued.
    control: Mutex<VecDeque<(Vec<u8>, Instant)>>,
    feedback_stats: Arc<super::feedback_stats::FeedbackStats>,
}

impl PeerLink {
    #[must_use]
    pub fn new(
        route_blob: &[u8],
        padding_key: super::roster::PaddingKey,
        feedback_stats: Arc<super::feedback_stats::FeedbackStats>,
    ) -> Arc<Self> {
        Arc::new(Self {
            controller: Mutex::new(RouteController::new()),
            padding_key,
            route: Mutex::new(route_blob.to_vec()),
            wake: Notify::new(),
            stop: CancellationToken::new(),
            sent: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            messages: AtomicU64::new(0),
            control: Mutex::new(VecDeque::new()),
            feedback_stats,
        })
    }

    /// Queue a control datagram (transport feedback, a receiver report) to
    /// ride the next media message, or go alone after
    /// [`CONTROL_ATTACH_WAIT`].
    pub fn enqueue_control(&self, wire: Vec<u8>) {
        self.control.lock().push_back((wire, Instant::now()));
        self.wake.notify_one();
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

    /// Run the driver until [`Self::stop`] or the scope's cancellation.
    pub async fn drive(
        self: Arc<Self>,
        peer: String,
        sender: Arc<dyn VoiceFrameSender>,
        allocator: Arc<Allocator>,
        scope_stop: CancellationToken,
    ) {
        let mut streak = 0u64;
        let mut last_stats = Instant::now();
        // How long each `app_message` took to hand off, since the last
        // stats line: while one is in flight, everything queued behind it
        // (voice included) waits.
        let mut send_times = SendTimes::default();
        // A datagram polled into a full bundle opens the next message.
        let mut carry: Option<Vec<u8>> = None;
        loop {
            let now = Instant::now();
            let (mut bundle, next, keyframe, stats) = {
                let mut c = self.controller.lock();
                let bundle = self.collect_bundle(&mut c, now, &mut carry);
                let stats = (now.duration_since(last_stats) >= STATS_EVERY).then(|| c.stats(now));
                (bundle, c.poll_timeout(), c.take_keyframe_wanted(), stats)
            };
            if keyframe {
                allocator.request_keyframe();
            }
            if let Some(stats) = stats {
                last_stats = now;
                self.log_stats(&peer, &stats, &mut send_times);
            }
            let control = self.attach_control(now, &mut bundle);
            if !bundle.is_empty() {
                let seqs: Vec<u32> = bundle
                    .iter()
                    .filter_map(|d| crate::media_frame::split_sequenced(d).map(|(_, seq, _)| seq))
                    .collect();
                let datagrams = u64::try_from(seqs.len()).unwrap_or(u64::MAX);
                let message = crate::bundle::encode(&bundle);
                let route = self.route();
                let started = Instant::now();
                let result = sender.send_voice_frame(&route, message).await;
                let took = started.elapsed();
                send_times.note(took);
                for (kind, queued_at) in &control {
                    if *kind == crate::media_frame::FEEDBACK_TAG {
                        self.feedback_stats.note_handed(
                            &peer,
                            started.saturating_duration_since(*queued_at),
                            took,
                            result.is_ok(),
                        );
                    }
                }
                match result {
                    Ok(()) => {
                        let handed = Instant::now();
                        let mut c = self.controller.lock();
                        for seq in seqs {
                            c.on_handed_off(seq, handed);
                        }
                        drop(c);
                        self.sent.fetch_add(datagrams, Ordering::Relaxed);
                        self.messages.fetch_add(1, Ordering::Relaxed);
                        streak = 0;
                    }
                    Err(e) => {
                        self.failed.fetch_add(datagrams, Ordering::Relaxed);
                        streak += 1;
                        log_failure(&peer, streak, &e);
                    }
                }
                continue;
            }
            // Nothing to send now: wake for the controller, or for a
            // control datagram whose wait for media runs out.
            let next = [next, self.control_deadline()].into_iter().flatten().min();
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

    /// Everything the controller releases now, as one message's datagrams:
    /// a carried-over datagram first, then each poll while it fits.
    fn collect_bundle(
        &self,
        c: &mut RouteController,
        now: Instant,
        carry: &mut Option<Vec<u8>>,
    ) -> Vec<Vec<u8>> {
        let mut bundle = Vec::new();
        let mut bytes = crate::bundle::framing_bytes(0);
        if let Some(d) = carry.take() {
            if let Some((_, seq, _)) = crate::media_frame::split_sequenced(&d) {
                c.charge_message_overhead(seq, now);
            }
            bytes += 2 + d.len();
            bundle.push(d);
        }
        loop {
            c.handle_timeout(now);
            let Some(mut d) = c.poll_datagram(now, bundle.is_empty()) else {
                break;
            };
            if d.first() == Some(&crate::media_frame::PADDING_TAG) {
                let signed = self
                    .padding_key
                    .read()
                    .as_ref()
                    .is_some_and(|key| crate::media_frame::sign_padding(&mut d, key));
                if !signed {
                    // Unsigned padding would be dropped on arrival.
                    continue;
                }
            }
            if crate::bundle::fits(bytes, bundle.len(), d.len()) {
                bytes += 2 + d.len();
                bundle.push(d);
            } else {
                *carry = Some(d);
                break;
            }
        }
        bundle
    }

    /// Put waiting control datagrams in `bundle`: all that fit when media
    /// is going anyway, or alone once the oldest has waited
    /// [`CONTROL_ATTACH_WAIT`]. Returns each attached datagram's tag and
    /// when it was queued.
    fn attach_control(&self, now: Instant, bundle: &mut Vec<Vec<u8>>) -> Vec<(u8, Instant)> {
        let mut control = self.control.lock();
        let due = control
            .front()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= CONTROL_ATTACH_WAIT);
        if bundle.is_empty() && !due {
            return Vec::new();
        }
        let mut bytes =
            crate::bundle::framing_bytes(bundle.len()) + bundle.iter().map(Vec::len).sum::<usize>();
        let mut attached = Vec::new();
        while let Some((wire, _)) = control.front() {
            if !crate::bundle::fits(bytes, bundle.len(), wire.len()) {
                break;
            }
            let Some((wire, at)) = control.pop_front() else {
                break;
            };
            bytes += 2 + wire.len();
            attached.push((wire.first().copied().unwrap_or_default(), at));
            bundle.push(wire);
        }
        attached
    }

    /// When the oldest waiting control datagram must go, media or not.
    fn control_deadline(&self) -> Option<Instant> {
        self.control
            .lock()
            .front()
            .map(|(_, at)| *at + CONTROL_ATTACH_WAIT)
    }

    fn log_stats(&self, peer: &str, s: &RouteStats, send_times: &mut SendTimes) {
        let (sent, failed) = self.send_counts();
        let (send_p50_ms, send_p95_ms, send_max_ms) = send_times.take();
        tracing::info!(
            peer = %peer,
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
            voice_frames_per_message = s.voice_frames,
            datagrams = s.sent,
            messages = self.messages.load(Ordering::Relaxed),
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
