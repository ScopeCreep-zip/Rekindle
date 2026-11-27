use std::time::Duration;

use super::TokenBucket;

#[test]
fn token_bucket_sustains_100_msgs_per_second_target() {
    // Architecture §32 / Week 26: gossip throughput target is 100
    // messages/second sustained. The token bucket sized to that
    // rate should accept exactly that many over a one-second window
    // and reject the 101st without burst.
    let start = std::time::Instant::now();
    let mut bucket = TokenBucket::new(100, 100, start);
    let mut accepted = 0u32;
    for _ in 0..100 {
        if bucket.try_consume_at(1, start) {
            accepted += 1;
        }
    }
    assert_eq!(accepted, 100, "100 msg/sec target must accept full burst");
    assert!(
        !bucket.try_consume_at(1, start),
        "101st msg in same instant must be rejected"
    );
    // After 1s of refill we should be back to full capacity.
    assert!(bucket.try_consume_at(100, start + Duration::from_secs(1)));
}

#[test]
fn token_refill_restores_capacity_over_time() {
    let start = std::time::Instant::now();
    let mut bucket = TokenBucket::new(10, 10, start);

    assert!(bucket.try_consume_at(10, start));
    assert!(!bucket.try_consume_at(1, start));

    assert!(!bucket.try_consume_at(6, start + Duration::from_millis(500)));
    assert!(bucket.try_consume_at(5, start + Duration::from_millis(500)));

    assert!(bucket.try_consume_at(10, start + Duration::from_secs(2)));
}
