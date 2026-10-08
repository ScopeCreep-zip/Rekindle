//! One peer route's media egress: the bandwidth owner (plans E4.3.1,
//! E4.3.3).
//!
//! Every media datagram for a peer leaves through its [`RouteController`]:
//! voice, video-plane envelopes and the pacer's padding. The controller is
//! str0m's send side driven the way str0m's `Session` drives it
//! (`session.rs` `handle_timeout_bwe`, `update_queue_state`, `poll_packet`,
//! `configure_pacer`), with `rekindle-media-bwe` holding str0m's estimator
//! and pacer (ADR 0015):
//!
//! - **Queues.** Audio and video-plane control go in an unpaced queue,
//!   released ahead of everything else; video fragments go in a paced
//!   queue, which also generates the padding the pacer asks for. This is
//!   libwebrtc's priority order, audio before video before padding
//!   (`evidence/e4-3-bandwidth-owner-design.md` #1).
//! - **Sequence numbers.** `transport_seq` is stamped when the pacer
//!   releases a datagram, so it counts only what was sent (#2). It is
//!   kept as a u64 here and travels as its low 32 bits.
//! - **Feedback.** Each `TransportFeedback` entry is matched to the send
//!   record and handed to the estimator once, as str0m's
//!   `TwccSendRegister::apply_report` does: remote arrival times are laid
//!   on a timeline anchored at the first report, and a packet reported
//!   before is passed again only if it was reported lost then (#4, #5).
//! - **Queue-time bound.** Video still queued past [`QUEUE_LIMIT`] is
//!   dropped and a keyframe is wanted; delta frames are refused until the
//!   keyframe arrives (#7).
//!
//! Sizes given to the pacer and estimator are what the route carries: our
//! datagram plus Veilid's per-message cost ([`ROUTE_OVERHEAD_BYTES`]), as
//! libwebrtc counts transport overhead in its send-side estimate. The
//! measured per-kind cost ([`RouteController::media_share`]) converts an
//! allocation on the wire back into encoder bitrates.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rekindle_codec::capnp_codec::transport_feedback::{TransportFeedback, NOT_RECEIVED};
use rekindle_media_bwe::{
    Bitrate, Bwe, DataSize, LeakyBucketPacer, Pacer, PacerControl, QueueId, SendQueue,
    TwccClusterId, TwccPacketId, TwccRecvReport, TwccSendRecord,
};

use crate::media_frame;
use share::Share;

/// The estimate a route starts from: libwebrtc's `kDefaultStartBitrateBps`
/// (`api/transport/bitrate_settings.h`). str0m starts its pacer at twice
/// the initial estimate (`session.rs`), and so does [`RouteController::new`].
pub const START_ESTIMATE: Bitrate = Bitrate::bps(300_000);

/// Longest a video datagram may wait. libwebrtc's pacer drains toward
/// `kMaxExpectedQueueLength` (2 s, `modules/pacing/pacing_controller.h`);
/// what still waits past it is dropped for a keyframe.
pub const QUEUE_LIMIT: Duration = Duration::from_secs(2);

/// How long a sent datagram stays matchable against feedback. Feedback
/// comes every 50–250 ms; a report about older packets is past use.
const HISTORY_WINDOW: Duration = Duration::from_secs(10);

/// Veilid's per-message cost on the first hop, beyond our datagram, for a
/// media `app_message` in steady state (veilid-core, our fork): the
/// recipient's private route travels in every message (976 B measured for
/// a media route; its entry hop is always a full PeerInfo,
/// `route_assemble.rs`), three safety-route hops of AEAD and `RouteHop`
/// framing (~246 B), the outer and inner operation framing and payload
/// AEAD (~256 B), LZ4 (~11 B) and the ENV0 header and signature (170 B,
/// `crypto/envelope`). A statement draws no reply. Fixed per message: it
/// is why the route charges a 20 ms voice frame about 25 times its Opus
/// payload.
pub const ROUTE_OVERHEAD_BYTES: usize = 1_660;

/// Largest padding datagram payload: one full video fragment
/// (`rekindle_video::FRAGMENT_PAYLOAD_LIMIT`), so probes look to the route
/// like the media they stand in for.
const MAX_PADDING_BYTES: usize = 4 * 1024;

/// Media sent this recently keeps the route "active" for padding
/// (str0m's `has_active_outgoing_media`).
const MEDIA_ACTIVE: Duration = Duration::from_secs(1);

/// Video offered this recently lets the estimator probe for more.
const VIDEO_ACTIVE: Duration = Duration::from_secs(2);

/// Longest video waits for a voice batch due soon, so both ride one
/// message: one Opus frame.
const COALESCE_WAIT: Duration = Duration::from_millis(20);
/// What one message costs beyond its datagrams: the route, plus our
/// sequence header (`bundle` framing is 2 bytes a datagram on top).
const PER_MESSAGE_OVERHEAD_BYTES: usize = ROUTE_OVERHEAD_BYTES + media_frame::SEQUENCED_HEADER_LEN;

const UNPACED_QUEUE: QueueId = QueueId(0);
const VIDEO_QUEUE: QueueId = QueueId(1);

/// One encoded video frame, ready for every peer's queue.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub keyframe: bool,
    /// Signed fragment and parity envelopes, in send order.
    pub fragments: Vec<Arc<[u8]>>,
    /// Encoder output bytes the fragments carry.
    pub media_bytes: usize,
}

/// A datagram payload waiting for the pacer.
#[derive(Debug)]
struct Queued {
    tag: u8,
    payload: Arc<[u8]>,
    /// Encoder bytes this datagram accounts for (0 for control and for a
    /// frame's later fragments).
    media_bytes: usize,
}

/// What feedback has said about one sent datagram.
#[derive(Debug, Clone, Copy)]
struct Reported {
    remote: Option<Instant>,
    /// The feedback round that last handed it to the estimator.
    round: u64,
}

#[derive(Debug)]
struct SentRecord {
    seq: u64,
    sent_at: Instant,
    size: usize,
    cluster: Option<TwccClusterId>,
    report: Option<Reported>,
}

/// Bytes the route carries for a datagram of `len` bytes.
fn wire_size(len: usize) -> usize {
    len + ROUTE_OVERHEAD_BYTES
}

/// The low 32 bits that travel on the wire.
fn wire_seq(seq: u64) -> u32 {
    u32::try_from(seq & u64::from(u32::MAX)).unwrap_or_default()
}

/// What the route measured of each media kind's wire cost, once it has
/// sent some: the allocator's conversion between wire and encoder rates.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediaShare {
    /// Wire bytes per voice packet beyond the Opus payload. Voice runs at
    /// a fixed packet rate, so its cost is linear in this.
    pub audio_overhead_bytes: Option<f64>,
    /// Encoder bytes per wire byte for video (fragmentation, parity,
    /// signatures and the route).
    pub video_share: Option<f64>,
}

/// The bandwidth owner of one peer route.
pub struct RouteController {
    bwe: Bwe,
    pacer: LeakyBucketPacer,
    unpaced: SendQueue<Queued>,
    video: SendQueue<Queued>,
    /// Padding bytes the pacer asked for and not yet sent.
    padding: usize,
    next_seq: u64,
    history: VecDeque<SentRecord>,
    /// Local instant the remote timeline is laid on (first feedback).
    time_zero: Option<Instant>,
    round: u64,
    /// Video was dropped (or the route is new): refuse delta frames until
    /// a keyframe.
    awaiting_keyframe: bool,
    /// A keyframe should be requested from the local encoder.
    keyframe_wanted: bool,
    last_media: Option<Instant>,
    last_video: Option<Instant>,
    audio_share: Share,
    video_share: Share,
    dropped_video: u64,
    /// Feedback seen since the last stats: reports, received, lost.
    window_feedback: (u64, u64, u64),
    /// Voice held so several frames ride one message (plan E4.3 T3).
    voice: voice_batch::VoiceBatcher,
}

impl Default for RouteController {
    fn default() -> Self {
        Self::new()
    }
}

impl RouteController {
    #[must_use]
    pub fn new() -> Self {
        let mut pacer = LeakyBucketPacer::new(START_ESTIMATE * 2.0);
        pacer.set_queue_limit(QUEUE_LIMIT);
        Self {
            bwe: Bwe::new(START_ESTIMATE),
            pacer,
            unpaced: SendQueue::new(),
            video: SendQueue::new(),
            padding: 0,
            next_seq: 0,
            history: VecDeque::new(),
            time_zero: None,
            round: 0,
            // A new route's receiver can decode nothing before a keyframe
            // (RFC 5104 FIR semantics for a new participant).
            awaiting_keyframe: true,
            keyframe_wanted: true,
            last_media: None,
            last_video: None,
            audio_share: Share::default(),
            video_share: Share::default(),
            dropped_video: 0,
            window_feedback: (0, 0, 0),
            voice: voice_batch::VoiceBatcher::default(),
        }
    }

    /// Queue a voice packet (`VOICE_TAG`) or a video-plane control envelope
    /// (`ENVELOPE_TAG`) ahead of everything paced.
    pub fn enqueue_unpaced(
        &mut self,
        tag: u8,
        payload: Arc<[u8]>,
        media_bytes: usize,
        now: Instant,
    ) {
        let size = wire_size(media_frame::SEQUENCED_HEADER_LEN + payload.len());
        let queued = Queued {
            tag,
            payload,
            media_bytes,
        };
        if tag == media_frame::VOICE_TAG {
            self.voice.push(queued, size, now);
        } else {
            self.unpaced.push(queued, size);
        }
        self.last_media = Some(now);
    }

    /// Queue a video frame's fragments behind audio. A delta frame is
    /// refused (and counted as dropped) while a keyframe is awaited.
    /// Returns whether it queued.
    pub fn enqueue_video(&mut self, frame: &VideoFrame, now: Instant) -> bool {
        self.last_video = Some(now);
        if self.awaiting_keyframe && !frame.keyframe {
            self.dropped_video += 1;
            return false;
        }
        self.awaiting_keyframe = false;
        for (i, fragment) in frame.fragments.iter().enumerate() {
            let size = wire_size(media_frame::SEQUENCED_HEADER_LEN + fragment.len());
            self.video.push(
                Queued {
                    tag: media_frame::ENVELOPE_TAG,
                    payload: Arc::clone(fragment),
                    media_bytes: if i == 0 { frame.media_bytes } else { 0 },
                },
                size,
            );
        }
        self.last_media = Some(now);
        true
    }

    /// Whether video was offered recently: the allocator then includes it
    /// in the rate the estimator probes toward.
    #[must_use]
    pub fn video_offered(&self, now: Instant) -> bool {
        self.recent(self.last_video, VIDEO_ACTIVE, now)
    }

    /// Whether the route wants a keyframe from the local encoder; clears
    /// the request.
    pub fn take_keyframe_wanted(&mut self) -> bool {
        std::mem::take(&mut self.keyframe_wanted)
    }

    /// The bitrate the senders on this route would use unconstrained, so
    /// the estimator knows how far to probe (str0m
    /// `set_bwe_desired_bitrate`).
    pub fn set_desired_bitrate(&mut self, desired: Bitrate, now: Instant) {
        self.bwe.set_desired_bitrate(desired);
        self.configure_pacer(now);
    }

    /// Advance timers: probes, queue timestamps, the queue-time bound and
    /// the pacer's padding request (str0m `handle_timeout_bwe` then
    /// `update_queue_state`).
    pub fn handle_timeout(&mut self, now: Instant) {
        for (queued, size) in self.voice.release(now) {
            self.unpaced.push(queued, size);
        }
        // str0m probes whenever a sending stream can carry probes, audio
        // included; here every queue can, so any recent media allows it.
        // Voice alone costs more on a Veilid route than the start estimate.
        let do_probe = self.recent(self.last_media, MEDIA_ACTIVE, now)
            || self.recent(self.last_video, VIDEO_ACTIVE, now);
        if let Some(config) = self.bwe.handle_timeout(now, do_probe) {
            // Only start the probe in the pacer if the estimator accepted it.
            if self.bwe.start_probe(config, now) {
                self.pacer.start_probe(config);
            }
        }
        if let Some(cluster) = self.pacer.check_probe_complete(now) {
            self.bwe.end_probe(now, cluster);
        }

        self.unpaced.handle_timeout(now);
        self.video.handle_timeout(now);
        self.drop_stale_video(now);

        let states = [self.unpaced_state(now), self.video_state(now)];
        if let Some(request) = self.pacer.handle_timeout(now, states.into_iter()) {
            if request.queue_id == VIDEO_QUEUE {
                self.padding += request.padding;
            }
        }
    }

    /// The next datagram the pacer releases, framed with its sequence
    /// number and recorded as sent at `now` (str0m `poll_packet`). Call
    /// [`Self::handle_timeout`] before each poll. `first_in_message`: the
    /// datagram opens a Veilid message and carries its route overhead; a
    /// later datagram in the same bundle costs only its bundle framing.
    pub fn poll_datagram(&mut self, now: Instant, first_in_message: bool) -> Option<Vec<u8>> {
        let (queue, cluster) = self.pacer.poll_queue()?;
        let (tag, payload, media_bytes, is_padding) = if queue == UNPACED_QUEUE {
            let q = self.unpaced.pop(now)?;
            (q.tag, q.payload, q.media_bytes, false)
        } else if let Some(q) = self.video.pop(now) {
            (q.tag, q.payload, q.media_bytes, false)
        } else if self.padding > 0 {
            // At least the signed header (`media_frame::sign_padding`),
            // as str0m pads to at least one SRTP block.
            let n = self
                .padding
                .clamp(media_frame::PADDING_HEADER_LEN, MAX_PADDING_BYTES);
            self.padding = self.padding.saturating_sub(n);
            (media_frame::PADDING_TAG, Arc::from(vec![0u8; n]), 0, true)
        } else {
            return None;
        };

        let seq = self.next_seq;
        self.next_seq += 1;
        let datagram = media_frame::sequenced(tag, wire_seq(seq), &payload);
        let size = if first_in_message {
            wire_size(datagram.len())
        } else {
            datagram.len() + 2
        };
        self.pacer.register_send(now, DataSize::from(size), queue);
        self.bwe
            .on_media_sent(DataSize::from(size), is_padding, now);

        while self
            .history
            .front()
            .is_some_and(|r| now.saturating_duration_since(r.sent_at) > HISTORY_WINDOW)
        {
            self.history.pop_front();
        }
        self.history.push_back(SentRecord {
            seq,
            sent_at: now,
            size,
            cluster,
            report: None,
        });
        match tag {
            media_frame::VOICE_TAG => self.audio_share.note(now, media_bytes, size),
            media_frame::ENVELOPE_TAG if queue == VIDEO_QUEUE => {
                self.video_share.note(now, media_bytes, size);
            }
            _ => {}
        }
        Some(datagram)
    }

    /// When [`Self::handle_timeout`] is next due.
    #[must_use]
    pub fn poll_timeout(&self) -> Option<Instant> {
        let (pacer_at, _) = self.pacer.poll_timeout();
        let (bwe_at, _) = self.bwe.poll_timeout();
        [pacer_at, bwe_at, self.voice.deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    /// Whether video should wait for the voice batch due within
    /// [`COALESCE_WAIT`], so both go in one message.
    pub(super) fn video_waits_for_voice(&self, now: Instant) -> bool {
        self.voice
            .deadline()
            .is_some_and(|d| d > now && d <= now + COALESCE_WAIT)
    }

    /// Voice frames per message on this route.
    #[must_use]
    pub fn voice_frames(&self) -> usize {
        self.voice.frames()
    }

    /// The datagram numbered `wire_seq_sent`, polled as part of a bundle,
    /// did not fit and opens the next message: charge it the route
    /// overhead it was not charged.
    pub fn charge_message_overhead(&mut self, wire_seq_sent: u32, now: Instant) {
        let extra = ROUTE_OVERHEAD_BYTES - 2;
        if let Some(record) = self
            .history
            .iter_mut()
            .rev()
            .take(64)
            .find(|r| wire_seq(r.seq) == wire_seq_sent)
        {
            record.size += extra;
        }
        self.pacer
            .register_send(now, DataSize::from(extra), UNPACED_QUEUE);
        self.bwe.on_media_sent(DataSize::from(extra), false, now);
    }

    /// The datagram numbered `wire_seq` was handed to Veilid at `at`: its
    /// send time is then, as libwebrtc stamps it at `OnSentPacket`, not
    /// when the pacer released it (the hand-off took up to 57 ms on Pop in
    /// call 1). Sends leave in sequence, so the record is near the back.
    pub fn on_handed_off(&mut self, wire_seq_sent: u32, at: Instant) {
        if let Some(record) = self
            .history
            .iter_mut()
            .rev()
            .take(64)
            .find(|r| wire_seq(r.seq) == wire_seq_sent)
        {
            record.sent_at = at;
        }
    }

    /// The route's round trip from the voice receiver reports, for the
    /// estimator's rate control (libwebrtc feeds AIMD the RTCP RTT).
    pub fn set_rtt(&mut self, rtt: Duration) {
        self.bwe.set_rtt(rtt);
    }

    /// Hand a feedback report from this route's receiver to the estimator
    /// and reconfigure the pacer (str0m `apply_report`, `bwe.update`,
    /// `configure_pacer`). Returns the estimate after it.
    pub fn on_feedback(&mut self, feedback: &TransportFeedback, now: Instant) -> Bitrate {
        let records = self.apply_report(feedback, now);
        if !records.is_empty() {
            self.bwe.update(records.iter(), now);
        }
        self.configure_pacer(now);
        let frames = super::allocation::voice_frames_per_message(
            self.estimate(),
            self.media_share().video_share,
            crate::transport::egress::share::as_f64(PER_MESSAGE_OVERHEAD_BYTES),
        );
        self.voice.set_frames(frames);
        self.estimate()
    }

    fn apply_report(&mut self, feedback: &TransportFeedback, now: Instant) -> Vec<TwccSendRecord> {
        let time_zero = *self.time_zero.get_or_insert(now);
        self.round += 1;
        self.window_feedback.0 += 1;
        let round = self.round;
        let Some(front_wire) = self.history.front().map(|r| wire_seq(r.seq)) else {
            return Vec::new();
        };
        let report_us = feedback.report_time_ms.saturating_mul(1_000);
        let mut records = Vec::new();
        for (i, &arrival) in feedback.arrivals.iter().enumerate() {
            let Ok(i) = u32::try_from(i) else { break };
            // History sequence numbers are consecutive, so the entry sits at
            // its wire distance from the front. A report about packets older
            // than the front wraps past the end and is skipped.
            let offset = feedback.begin_seq.wrapping_add(i).wrapping_sub(front_wire);
            let Some(record) = usize::try_from(offset)
                .ok()
                .and_then(|o| self.history.get_mut(o))
            else {
                continue;
            };
            let remote = (arrival != NOT_RECEIVED).then(|| {
                time_zero
                    + Duration::from_micros(
                        report_us.saturating_sub(u64::from(arrival) * 1_000_000 / 1_024),
                    )
            });
            // A packet already acked keeps its first arrival and is not
            // handed over again; one reported lost is handed over again
            // with whatever this report says.
            let (remote, handed_round) = match record.report {
                Some(Reported {
                    remote: Some(t),
                    round: r,
                }) => (Some(t), r),
                _ => (remote, round),
            };
            record.report = Some(Reported {
                remote,
                round: handed_round,
            });
            if handed_round == round {
                if remote.is_some() {
                    self.window_feedback.1 += 1;
                } else {
                    self.window_feedback.2 += 1;
                }
                let id = record.cluster.map_or_else(
                    || TwccPacketId::new(record.seq),
                    |c| TwccPacketId::with_cluster(record.seq, c),
                );
                records.push(TwccSendRecord::new(
                    id,
                    record.sent_at,
                    record.size,
                    Some(TwccRecvReport::new(now, remote)),
                ));
            }
        }
        records
    }

    /// The route's current estimate on the wire.
    #[must_use]
    pub fn estimate(&self) -> Bitrate {
        self.bwe.last_estimate().unwrap_or(START_ESTIMATE)
    }

    /// The wire cost of each media kind, once measured.
    #[must_use]
    pub fn media_share(&self) -> MediaShare {
        MediaShare {
            audio_overhead_bytes: self.audio_share.overhead_per_packet(),
            video_share: self.video_share.ratio(),
        }
    }

    fn configure_pacer(&mut self, now: Instant) {
        let Some(estimate) = self.bwe.last_estimate() else {
            // No estimate yet, no padding (str0m).
            return;
        };
        let active = self.recent(self.last_media, MEDIA_ACTIVE, now);
        let rates = PacerControl::calculate(active, estimate, self.bwe.is_overusing());
        self.pacer.set_padding_rate(rates.padding_rate);
        self.pacer.set_pacing_rate(rates.pacing_rate);
    }

    fn recent(&self, at: Option<Instant>, within: Duration, now: Instant) -> bool {
        let _ = self;
        at.is_some_and(|t| now.saturating_duration_since(t) <= within)
    }

    fn drop_stale_video(&mut self, now: Instant) {
        let Some(first) = self.video.snapshot(now).first_unsent else {
            return;
        };
        if now.saturating_duration_since(first) <= QUEUE_LIMIT {
            return;
        }
        tracing::debug!(
            waited_ms = now.saturating_duration_since(first).as_millis(),
            "video past its queue-time bound: dropped, keyframe wanted"
        );
        self.drop_video();
        self.keyframe_wanted = true;
    }

    fn drop_video(&mut self) {
        if !self.video.is_empty() {
            self.video.clear();
            self.dropped_video += 1;
        }
        self.awaiting_keyframe = true;
    }
}

mod queue_state;
mod share;
mod stats;
mod voice_batch;
pub use stats::RouteStats;
#[cfg(test)]
mod tests;
