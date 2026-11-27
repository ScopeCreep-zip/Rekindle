use super::*;

#[test]
fn peek_pop_no_timestamp() {
    let mut queue = SendQueue::new();

    queue.push(0_u64, 0);

    assert!(queue.peek().is_none());
    assert!(queue.pop(Instant::now()).is_none());
    assert!(queue.need_timeout());

    let snapshot_at = Instant::now() + Duration::from_secs(3);
    assert_eq!(
        queue.snapshot(snapshot_at),
        QueueSnapshot {
            created_at: snapshot_at,
            packet_count: 0,
            byte_size: 0,
            total_queue_time_origin: Duration::ZERO,
            first_unsent: None,
            priority: QueuePriority::Empty,
            ..Default::default()
        }
    );
}

#[test]
fn peek_pop_after_timestamp() {
    let mut queue = SendQueue::new();

    let start = Instant::now();

    queue.push(0_u64, 2);

    queue.handle_timeout(start);

    assert!(queue.peek().is_some());
    assert!(!queue.need_timeout());

    let snapshot_at = start + Duration::from_secs(3);
    assert_eq!(
        queue.snapshot(snapshot_at),
        QueueSnapshot {
            created_at: snapshot_at,
            packet_count: 1,
            byte_size: 2,
            total_queue_time_origin: Duration::from_secs(3),
            first_unsent: Some(start),
            priority: QueuePriority::Media,
            ..Default::default()
        }
    );

    assert!(queue.pop(Instant::now()).is_some());
}

#[test]
fn untimed_packets_are_contiguous_at_tail() {
    let mut queue = SendQueue::new();
    let start = Instant::now();

    queue.push(0_u64, 10);
    queue.push(1_u64, 10);
    queue.push(2_u64, 10);

    assert!(
        queue.queue.iter().all(|q| q.timestamp == not_happening()),
        "expect every new packet's timestamp to start with the sentinel value"
    );
    assert!(queue.need_timeout());

    let now = start + Duration::from_millis(10);
    queue.handle_timeout(now);

    assert!(
        queue.queue.iter().all(|q| q.timestamp != not_happening()),
        "expect handle_timeout to have timestamped every packet"
    );
    assert!(!queue.need_timeout());
}

#[test]
fn total_queue() {
    let mut total_queue = TotalQueue::default();
    let now = Instant::now();
    total_queue.increase(now, 0);
    total_queue.increase(now, 1);
    total_queue.decrease(now, 1, Duration::ZERO);
    // Doesn't panic
    total_queue.move_time_forward(now + Duration::from_millis(1));
}
