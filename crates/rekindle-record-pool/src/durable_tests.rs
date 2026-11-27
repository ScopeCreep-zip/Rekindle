use super::*;

#[test]
fn backoff_doubles_from_two_seconds_to_a_minute() {
    assert_eq!(backoff(0), Duration::from_secs(2));
    assert_eq!(backoff(1), Duration::from_secs(4));
    assert_eq!(backoff(4), Duration::from_secs(32));
    assert_eq!(backoff(5), Duration::from_secs(60));
    assert_eq!(backoff(30), Duration::from_secs(60));
}
