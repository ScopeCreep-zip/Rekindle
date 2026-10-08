use super::share::as_f64;
use super::*;

const AUDIO_PAYLOAD: usize = 200;

/// A controller whose allocation has room for video.
fn with_video() -> RouteController {
    let mut c = RouteController::new();
    c.set_video_allowed(true);
    c.take_keyframe_wanted();
    c
}

fn drive(c: &mut RouteController, now: Instant) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        c.handle_timeout(now);
        match c.poll_datagram(now) {
            Some(d) => out.push(d),
            None => return out,
        }
    }
}

fn audio(c: &mut RouteController, now: Instant) {
    c.enqueue_unpaced(
        media_frame::VOICE_TAG,
        Arc::from(vec![1u8; AUDIO_PAYLOAD]),
        80,
        now,
    );
}

fn frame(keyframe: bool, fragments: usize, size: usize) -> VideoFrame {
    VideoFrame {
        keyframe,
        fragments: (0..fragments).map(|_| Arc::from(vec![2u8; size])).collect(),
        media_bytes: fragments * size,
    }
}

fn feedback(begin_seq: u32, report_time_ms: u64, arrivals: Vec<u16>) -> TransportFeedback {
    TransportFeedback {
        reporter_key: vec![0; 32],
        begin_seq,
        report_time_ms,
        arrivals,
        sig: Vec::new(),
    }
}

#[test]
fn audio_leaves_ahead_of_queued_video() {
    let mut c = with_video();
    let t0 = Instant::now();
    assert!(c.enqueue_video(&frame(true, 20, 4096), t0));
    audio(&mut c, t0);
    let sent = drive(&mut c, t0);
    let tags: Vec<u8> = sent.iter().map(|d| d[0]).collect();
    let first_audio = tags.iter().position(|&t| t == media_frame::VOICE_TAG);
    assert!(
        first_audio.is_some_and(|i| i <= 1),
        "audio is released at once (after at most the first-ever packet): {tags:?}"
    );
    assert!(
        tags.len() < 21,
        "the paced video is not all released in one instant"
    );
}

#[test]
fn sequence_numbers_are_consecutive_and_wrap_on_the_wire() {
    let mut c = RouteController::new();
    c.next_seq = u64::from(u32::MAX);
    let t0 = Instant::now();
    audio(&mut c, t0);
    audio(&mut c, t0);
    let sent = drive(&mut c, t0);
    let seqs: Vec<u32> = sent
        .iter()
        .map(|d| media_frame::split_sequenced(d).unwrap().1)
        .collect();
    assert_eq!(seqs, vec![u32::MAX, 0]);
    // Feedback in wire numbers finds both records across the wrap.
    let records = c.apply_report(&feedback(u32::MAX, 1_000, vec![10, 0]), t0);
    assert_eq!(records.len(), 2);
    assert_eq!(*records[1].seq(), u64::from(u32::MAX) + 1);
}

#[test]
fn a_packet_is_handed_to_the_estimator_once_unless_it_was_reported_lost() {
    let mut c = RouteController::new();
    let t0 = Instant::now();
    for _ in 0..3 {
        audio(&mut c, t0);
    }
    assert_eq!(drive(&mut c, t0).len(), 3);
    let first = c.apply_report(&feedback(0, 1_000, vec![20, NOT_RECEIVED]), t0);
    assert_eq!(first.len(), 2);
    assert!(first[1].remote_recv_time().is_none());
    // Overlapping report: seq 0 was acked (skipped), seq 1 was lost and now
    // arrived, seq 2 is new.
    let second = c.apply_report(&feedback(0, 1_100, vec![120, 50, 0]), t0);
    let seqs: Vec<u64> = second.iter().map(|r| *r.seq()).collect();
    assert_eq!(seqs, vec![1, 2]);
    assert!(second.iter().all(|r| r.remote_recv_time().is_some()));
    // A report about packets before the history is ignored.
    assert!(c
        .apply_report(&feedback(u32::MAX - 5, 1_200, vec![0; 3]), t0)
        .is_empty());
}

#[test]
fn remote_times_keep_the_receivers_spacing() {
    let mut c = RouteController::new();
    let t0 = Instant::now();
    audio(&mut c, t0);
    audio(&mut c, t0);
    drive(&mut c, t0);
    // Arrived 1024/1024 s and 0 s before a report at 5 s.
    let r = c.apply_report(&feedback(0, 5_000, vec![1_024, 0]), t0);
    let a = r[0].remote_recv_time().unwrap();
    let b = r[1].remote_recv_time().unwrap();
    assert_eq!(b - a, Duration::from_secs(1));
}

#[test]
fn stale_video_is_dropped_and_deltas_wait_for_a_keyframe() {
    let mut c = with_video();
    let t0 = Instant::now();
    assert!(c.enqueue_video(&frame(true, 5, 4096), t0));
    c.handle_timeout(t0);
    let late = t0 + QUEUE_LIMIT + Duration::from_millis(100);
    c.handle_timeout(late);
    assert_eq!(c.stats(late).dropped_video, 1);
    assert!(c.take_keyframe_wanted());
    assert!(!c.take_keyframe_wanted(), "the request is taken once");
    assert!(!c.enqueue_video(&frame(false, 1, 100), late));
    assert!(c.enqueue_video(&frame(true, 1, 100), late));
    assert!(c.enqueue_video(&frame(false, 1, 100), late));
}

#[test]
fn a_paused_route_refuses_video_and_resumes_on_a_keyframe() {
    let mut c = with_video();
    let t0 = Instant::now();
    assert!(c.enqueue_video(&frame(true, 2, 1000), t0));
    c.set_video_allowed(false);
    assert!(c.video.is_empty(), "pausing drops what was queued");
    assert!(!c.enqueue_video(&frame(true, 1, 100), t0));
    c.set_video_allowed(true);
    assert!(c.take_keyframe_wanted());
    assert!(!c.enqueue_video(&frame(false, 1, 100), t0));
    assert!(c.enqueue_video(&frame(true, 1, 100), t0));
}

/// A bottleneck link: FIFO at `capacity_bps` plus `base` one-way delay,
/// with the receiver reporting every 100 ms over the same base delay.
struct Link {
    capacity_bps: f64,
    base: Duration,
    free_at: Instant,
    /// (wire seq, arrival)
    arrivals: Vec<(u32, Instant)>,
    reported: usize,
}

impl Link {
    fn carry(&mut self, datagram: &[u8], sent: Instant) {
        let bits = as_f64(wire_size(datagram.len())) * 8.0;
        let start = self.free_at.max(sent);
        self.free_at = start + Duration::from_secs_f64(bits / self.capacity_bps);
        let seq = media_frame::split_sequenced(datagram).unwrap().1;
        self.arrivals.push((seq, self.free_at + self.base));
    }

    /// The report the receiver makes at `t`, about what arrived by then.
    fn report(&mut self, t: Instant, origin: Instant) -> Option<TransportFeedback> {
        let arrived: Vec<(u32, Instant)> = self.arrivals[self.reported..]
            .iter()
            .take_while(|(_, at)| *at <= t)
            .copied()
            .collect();
        let first = arrived.first()?.0;
        self.reported += arrived.len();
        let report_ms = u64::try_from((t - origin).as_millis()).unwrap();
        let report_at = origin + Duration::from_millis(report_ms);
        let offsets = arrived
            .iter()
            .map(|(_, at)| {
                let before = report_at.saturating_duration_since(*at);
                u16::try_from(before.as_micros() * 1_024 / 1_000_000)
                    .unwrap_or(rekindle_codec::capnp_codec::transport_feedback::MAX_ARRIVAL_OFFSET)
            })
            .collect();
        Some(feedback(first, report_ms, offsets))
    }
}

/// Run audio (50 pps) and video offered at `video_bps` over `link` for
/// `secs`; returns the final estimate.
fn simulate(capacity_bps: f64, video_bps: u32, secs: u64) -> Bitrate {
    let origin = Instant::now();
    let mut c = with_video();
    c.set_desired_bitrate(Bitrate::bps(4_000_000), origin);
    let mut link = Link {
        capacity_bps,
        base: Duration::from_millis(100),
        free_at: origin,
        arrivals: Vec::new(),
        reported: 0,
    };
    let mut pending_feedback: VecDeque<(Instant, TransportFeedback)> = VecDeque::new();
    let step = Duration::from_millis(5);
    let mut now = origin;
    let end = origin + Duration::from_secs(secs);
    let mut tick = 0u64;
    while now < end {
        if tick.is_multiple_of(4) {
            audio(&mut c, now);
        }
        if tick.is_multiple_of(13) {
            // ~15 fps
            let bytes = usize::try_from(video_bps / 15 / 8).unwrap();
            let n = bytes.div_ceil(4096).max(1);
            c.enqueue_video(&frame(false, n, bytes / n), now);
            c.awaiting_keyframe = false;
        }
        for d in drive(&mut c, now) {
            link.carry(&d, now);
        }
        if tick.is_multiple_of(20) {
            if let Some(fb) = link.report(now, origin) {
                pending_feedback.push_back((now + link.base, fb));
            }
        }
        while pending_feedback.front().is_some_and(|(at, _)| *at <= now) {
            let (_, fb) = pending_feedback.pop_front().unwrap();
            c.on_feedback(&fb, now);
        }
        now += step;
        tick += 1;
    }
    c.estimate()
}

#[test]
fn the_estimate_climbs_on_a_fast_link() {
    // GoogCC settles just under capacity: ~5.7 of 6 Mbps after 30 s.
    let est = simulate(6_000_000.0, 2_000_000, 30);
    assert!(
        est > Bitrate::bps(4_500_000),
        "estimate {est} should climb from the 300 kbps start toward capacity"
    );
}

#[test]
fn the_estimate_stays_under_a_slow_link() {
    // ~1.2 Mbps on a 1.5 Mbps bottleneck: under it, and not collapsed.
    let est = simulate(1_500_000.0, 2_000_000, 30);
    assert!(
        est > Bitrate::bps(900_000) && est < Bitrate::bps(1_500_000),
        "estimate {est} should sit just under a 1.5 Mbps bottleneck"
    );
}
