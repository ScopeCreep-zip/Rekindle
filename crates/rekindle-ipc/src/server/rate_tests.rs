use super::{Lane, RateBuckets, RATE_LIMIT_REFILL_MS};

#[test]
fn the_frame_bucket_admits_240_a_second_and_never_spends_a_request_token() {
    let buckets = RateBuckets::new();
    let now = 10_000;
    for i in 0..240 {
        assert!(buckets.admit(Lane::MediaUp, now), "frame {i} within 240/s");
    }
    assert!(
        !buckets.admit(Lane::MediaUp, now),
        "the 241st frame is dropped"
    );
    assert_eq!(buckets.media_up_dropped(), 1, "and counted");
    // The request bucket is untouched: all 100 requests still pass.
    for i in 0..100 {
        assert!(
            buckets.admit(Lane::Request, now),
            "request {i} within 100/s"
        );
    }
    assert!(!buckets.admit(Lane::Request, now));
    // Spent requests do not refuse frames either, and a second refills.
    assert!(!buckets.admit(Lane::MediaUp, now));
    assert_eq!(buckets.media_up_dropped(), 2);
    assert!(
        RateBuckets::new().admit(Lane::MediaUp, now),
        "a separate connection"
    );
    assert!(!buckets.admit(Lane::MediaUp, now + RATE_LIMIT_REFILL_MS - 1));
    assert!(buckets.admit(Lane::MediaUp, now + RATE_LIMIT_REFILL_MS));
    assert!(
        buckets.admit(Lane::DaemonMedia, now),
        "daemon media is exempt"
    );
}
