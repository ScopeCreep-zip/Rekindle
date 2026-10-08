//! Unit tests of the leaky-bucket pacer (str0m `src/pacer/leaky.rs`, `mod test`).

use super::super::{QueuePriority, QueueSnapshot};
use super::*;
use queue::{PacketKind, Queue, QueuedPacket};
use std::time::{Duration, Instant};

mod burst;
mod queue;
mod realistic;

/// Stands in for str0m's `RtpHeader` in these tests: only the sequence number
/// is read.
#[derive(Debug, Default, Clone, Copy)]
struct RtpHeader {
    sequence_number: u16,
}

#[test]
fn probe_deadline_without_media_or_regular_padding() {
    use crate::bwe::ProbeKind;

    let now = Instant::now();
    let mut pacer = LeakyBucketPacer::new(Bitrate::kbps(300));
    let queue = QueueState {
        queue_id: QueueId(0),
        unpaced: false,
        use_for_padding: true,
        snapshot: QueueSnapshot {
            created_at: now,
            ..Default::default()
        },
    };
    pacer.handle_timeout(now, std::iter::once(queue));
    pacer.start_probe(ProbeClusterConfig::new(
        0.into(),
        Bitrate::kbps(300),
        ProbeKind::Initial,
    ));
    let request = pacer.handle_timeout(now, std::iter::once(queue)).unwrap();
    let (queue_id, cluster) = pacer.poll_queue().unwrap();
    assert_eq!(cluster, Some(0.into()));
    pacer.register_send(now, request.padding.into(), queue_id);
    assert!(pacer.handle_timeout(now, std::iter::once(queue)).is_none());
    let deadline = pacer.poll_timeout().0.unwrap();
    assert!(deadline > now && deadline <= now + Duration::from_millis(10));
    assert!(pacer.poll_queue().is_none());
    assert!(pacer
        .handle_timeout(deadline, std::iter::once(queue))
        .is_some());
}

#[test]
fn test_typical_behavior() {
    let now = Instant::now();
    let mut queue = Queue::default();
    // 2,000 bits per second, 10 bytes per pacing interval(40ms)
    let mut pacer = LeakyBucketPacer::new((10 * 200).into());
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(1));

    assert!(
        pacer.poll_queue().is_none(),
        "We initially attempt to poll any non-empty queue if we have never sent",
    );

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        1,
        5,
        PacketKind::Video,
        now + duration_ms(21),
    );

    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(21),
        "First packet should be released because we have no debt",
        |packet| {
            assert_eq!(packet.header.sequence_number, 1);
        },
    );

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        2,
        8,
        PacketKind::Video,
        now + duration_ms(27),
    );
    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        3,
        25,
        PacketKind::Video,
        now + duration_ms(28),
    );

    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(28),
        "Second packet should be released because the debt is within tolerance",
        |packet| {
            assert_eq!(packet.header.sequence_number, 2);
        },
    );

    // We have incurred too much media debt so polling will now fail until the debt can be
    // reduced.
    assert!(
        pacer.poll_queue().is_none(),
        "Third packet should not be released because we have too much debt"
    );

    // Periodic timeout
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(41));

    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(41),
        "Third packet should be released because we have cleared debt as time moved forward",
        |packet| {
            assert_eq!(packet.header.sequence_number, 3);
        },
    );

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        4,
        12,
        PacketKind::Video,
        now + duration_ms(45),
    );
    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        5,
        25,
        PacketKind::Video,
        now + duration_ms(47),
    );

    // We have incurred too much media debt so polling will now fail until the debt can be
    // reduced.
    assert!(
        pacer.poll_queue().is_none(),
        "Fourth packet should not be released because we have too much debt"
    );

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        6,
        100,
        PacketKind::Audio,
        now + duration_ms(52),
    );

    // Unpaced packets should be able to send even if we have too much media debt.
    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(52),
        "Sixth packet (audio) should be released despite too much media debt because \
        audio packets are not paced",
        |packet| {
            assert_eq!(packet.kind, PacketKind::Audio);
            assert_eq!(packet.header.sequence_number, 6);
        },
    );

    // A lot of time passes, now the bitrate should be adjusted to force drain the queues to
    // avoid packets being queued for too long.
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(2053));

    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(2053),
        "Fourth packet should be released after hitting the queue limit",
        |packet| {
            assert_eq!(packet.header.sequence_number, 4);
        },
    );

    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(2053),
        "Fifth packet should be released after hitting the queue limit",
        |packet| {
            assert_eq!(packet.header.sequence_number, 5);
        },
    );

    assert!(queue.is_empty());
}

#[test]
fn test_queue_drain() {
    let now = Instant::now();
    let mut queue = Queue::default();
    // 2,000 bits per second, 10 bytes per pacing interval(40ms)
    let mut pacer = LeakyBucketPacer::new((10 * 200).into());
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(1));

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        1,
        21,
        PacketKind::Video,
        now + duration_ms(21),
    );

    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(21),
        "First packet should be released because we have no debt",
        |packet| {
            assert_eq!(packet.header.sequence_number, 1);
        },
    );

    // Time moves forward
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(41));

    // Nothing happens for a while because there's nothing in the queues.

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        2,
        8,
        PacketKind::Video,
        // Debt will be just slightly above what can be drained in 40 ms
        // after 66ms
        now + duration_ms(66),
    );

    assert!(
        pacer.poll_queue().is_none(),
        "Second packet should not be released because there's too much debt"
    );

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        3,
        5,
        PacketKind::Video,
        now + duration_ms(70),
    );
    // Drain packet 2
    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(70),
        "Second packet should be released because of the adjusted bitrate to drain the queue",
        |packet| {
            assert_eq!(packet.header.sequence_number, 2);
        },
    );

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        4,
        1200,
        PacketKind::Video,
        now + duration_ms(71),
    );

    // Drain packet 3
    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(71),
        "Third packet should be released because of the adjusted bitrate to drain the queue",
        |packet| {
            assert_eq!(packet.header.sequence_number, 3);
        },
    );

    // Drain packet 4
    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(71),
        "Fourth packet should be released because of the adjusted bitrate to drain the queue",
        |packet| {
            assert_eq!(packet.header.sequence_number, 4);
        },
    );

    // Time moves forward
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(81));

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        5,
        40,
        PacketKind::Video,
        now + duration_ms(81),
    );

    assert!(
        pacer.poll_queue().is_none(),
        "Fifth packet shoud not be relaesed because there's too much debt"
    );
}

#[test]
fn test_padding_fill_in() {
    let now = Instant::now();
    let mut queue = Queue::default();
    let mut pacer = LeakyBucketPacer::new((10 * 200).into());
    // 2,000 bits per second, 10 bytes per pacing interval(40ms) with padding at 3,000 bits per
    // second, 15 bytes per pacing interval(40ms)
    pacer.set_pacing_rate((10 * 200).into());
    pacer.set_padding_rate((15 * 200).into());
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(1));

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        1,
        21,
        PacketKind::Video,
        now + duration_ms(21),
    );

    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(21),
        "First packet should be released because we have no debt",
        |packet| {
            assert_eq!(packet.header.sequence_number, 1);
        },
    );

    // Time moves forward
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(41));

    // Nothing happens for a while because there's nothing in the queues.

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        2,
        8,
        PacketKind::Video,
        now + duration_ms(70),
    );

    // Drain packet 2
    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(70),
        "Second packet should be released because of the adjusted bitrate to drain the queue",
        |packet| {
            assert_eq!(packet.header.sequence_number, 2);
        },
    );

    // Time moves forward, all debt is cleared out now
    handle_timeout_noisy(&mut pacer, &mut queue, now + duration_ms(155));

    // Drain padding packet
    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(165),
        "The queued padding packet should be drained",
        |packet| {
            assert_eq!(packet.size(), 2);
            assert_eq!(packet.header.sequence_number, 0);
        },
    );

    enqueue_packet_noisy(
        &mut pacer,
        &mut queue,
        3,
        15,
        PacketKind::Video,
        now + duration_ms(165),
    );

    // Drain packet 3
    assert_poll_success(
        &mut pacer,
        &mut queue,
        now + duration_ms(165),
        "Third packet should be released because the sent padding doesn't \
        increase the media debt too much",
        |packet| {
            assert_eq!(packet.header.sequence_number, 3);
        },
    );
}

#[test]
fn test_queue_state_merge() {
    let now = Instant::now();

    let mut state = QueueState {
        queue_id: QueueId(1),
        unpaced: false,
        use_for_padding: true,
        snapshot: QueueSnapshot {
            created_at: now,
            byte_size: 10_usize,
            packet_count: 1332,
            total_queue_time_origin: duration_ms(1_000),
            last_emitted: Some(now + duration_ms(500)),
            first_unsent: None,
            priority: QueuePriority::Media,
        },
    };

    let other = QueueState {
        queue_id: QueueId(2),
        unpaced: false,
        use_for_padding: false,
        snapshot: QueueSnapshot {
            created_at: now,
            byte_size: 30_usize,
            packet_count: 5,
            total_queue_time_origin: duration_ms(337),
            last_emitted: None,
            first_unsent: Some(now + duration_ms(19)),
            priority: QueuePriority::Padding,
        },
    };

    state.snapshot.merge(&other.snapshot);

    assert_eq!(state.queue_id, QueueId(1));
    assert_eq!(state.snapshot.byte_size, 40_usize);
    assert_eq!(state.snapshot.packet_count, 1337);
    assert_eq!(state.snapshot.total_queue_time_origin, duration_ms(1337));

    assert_eq!(state.snapshot.last_emitted, Some(now + duration_ms(500)));
    assert_eq!(state.snapshot.first_unsent, Some(now + duration_ms(19)));
    assert_eq!(state.snapshot.priority, QueuePriority::Media);
}

#[test]
fn test_priority_ordering() {
    assert!(QueuePriority::Media < QueuePriority::Padding);
    assert!(QueuePriority::Media < QueuePriority::Empty);
    assert!(QueuePriority::Padding < QueuePriority::Empty);
}

fn assert_poll_success<F>(
    pacer: &mut impl Pacer,
    queue: &mut Queue,
    now: Instant,
    msg: &str,
    do_asserts: F,
) -> Instant
where
    F: Fn(QueuedPacket),
{
    let (qid, _cluster_id) = pacer.poll_queue().expect(msg);
    let packet = queue.next_packet().unwrap();
    let packet_size = packet.size();
    do_asserts(packet);
    pacer.register_send(now, DataSize::from(packet_size), qid);
    queue.register_send(qid, now);

    let timeout = pacer.poll_timeout().0;
    // After gating, the pacer requests a timeout at now + 1µs to ensure time advances
    const MINIMAL_DELTA: Duration = Duration::from_micros(1);
    assert!(
        timeout <= Some(now + MINIMAL_DELTA) && timeout.is_some(),
        "After a successful send the pacer should return an immediate timeout"
    );

    // Simulate an immediate call to handle_timeout
    handle_timeout_noisy(pacer, queue, now);

    timeout.unwrap()
}

fn enqueue_packet_noisy(
    pacer: &mut impl Pacer,
    queue: &mut Queue,
    seq_no: u16,
    size: usize,
    kind: PacketKind,
    now: Instant,
) {
    let (header, payload_len, kind) = make_packet(seq_no, size, kind);

    let queued_packet = QueuedPacket {
        queued_at: now,
        header,
        payload_len,
        kind,
    };
    queue.enqueue_packet(queued_packet);

    // Matches the queueing behavior when the pacer is used in real code.
    // Each packet being queued causes time to move forward in the pacer and the queue.
    handle_timeout_noisy(pacer, queue, now);
}

fn handle_timeout_noisy(pacer: &mut impl Pacer, queue: &mut Queue, now: Instant) {
    queue.update_average_queue_time(now);
    if let Some(padding_request) = pacer.handle_timeout(now, queue.queue_state(now)) {
        queue.generate_padding(padding_request.padding, now);

        let timeout = pacer.poll_timeout().0;
        if timeout.is_some_and(|t| t <= now) {
            // Refresh queue state
            pacer.handle_timeout(now, queue.queue_state(now));
        }
    }
}

fn duration_ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn make_packet(seq_no: u16, size: usize, kind: PacketKind) -> (RtpHeader, usize, PacketKind) {
    let header = RtpHeader {
        sequence_number: seq_no,
    };

    (header, size, kind)
}
