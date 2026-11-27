use super::*;

#[test]
fn backoff_schedule() {
    assert_eq!(backoff_duration(0), Duration::from_secs(1));
    assert_eq!(backoff_duration(1), Duration::from_secs(2));
    assert_eq!(backoff_duration(2), Duration::from_secs(4));
}
