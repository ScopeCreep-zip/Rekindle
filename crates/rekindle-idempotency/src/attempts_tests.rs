use super::*;

#[test]
fn four_failures_are_free_then_the_waits_escalate() {
    let limiter = AttemptLimiter::<&str>::default();
    for _ in 0..4 {
        assert_eq!(limiter.fail(&"me", 0), 0);
        assert!(limiter.check(&"me", 0).is_ok());
    }
    assert_eq!(limiter.fail(&"me", 0), MINUTE_MS);
    assert_eq!(limiter.check(&"me", 1_000), Err(MINUTE_MS - 1_000));
    assert!(limiter.check(&"me", MINUTE_MS).is_ok());
    assert_eq!(limiter.fail(&"me", MINUTE_MS), 5 * MINUTE_MS);
    assert_eq!(delay_after(8), 15 * MINUTE_MS);
    assert_eq!(delay_after(9), 60 * MINUTE_MS);
    assert!(limiter.check(&"other", 0).is_ok(), "per key");
    assert!(limiter.succeed(&"me") > 0);
    assert!(limiter.check(&"me", MINUTE_MS).is_ok());
}
