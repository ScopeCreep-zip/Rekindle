use super::*;

#[test]
fn re_sends_wait_a_minute_then_double_to_half_an_hour() {
    let waits: Vec<u64> = (0..8).map(|n| wait_after(n).as_secs()).collect();
    assert_eq!(waits, [60, 120, 240, 480, 960, 1800, 1800, 1800]);
    assert_eq!(wait_after(u32::MAX), CAP);
}

#[test]
fn re_sends_stop_once_a_day_has_passed() {
    let since = Instant::now();
    let (mut last, mut sent, mut times) = (since, 0, 0u32);
    while let Some(due) = next_due(since, sent, last) {
        assert!(due <= since + WINDOW);
        last = due;
        sent += 1;
        times += 1;
    }
    // 60+120+…+960 = 1860 s, then 30-min waits to the 24 h mark:
    // 5 + (86_400 − 1_860) / 1_800 = 5 + 46.
    assert_eq!(times, 51);
    assert!(next_due(since, sent, last).is_none());
    assert!(last + CAP > since + WINDOW);
}
