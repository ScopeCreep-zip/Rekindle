use super::*;

#[test]
fn empty_skeleton_has_30_oldest_first_buckets() {
    let now = 1_700_000_000_000_i64;
    let skel = empty_skeleton(now);
    assert_eq!(skel.len(), DAILY_TIMESERIES_DAYS as usize);
    for win in skel.windows(2) {
        assert_eq!(win[1].day_unix_ms - win[0].day_unix_ms, ONE_DAY_MS);
    }
    // Last bucket is today's midnight.
    assert_eq!(skel.last().unwrap().day_unix_ms, day_floor_ms(now));
}

#[test]
fn merge_fills_only_matching_days() {
    let now = 1_700_000_000_000_i64;
    let skel = empty_skeleton(now);
    let today = day_floor_ms(now);
    let yesterday = today - ONE_DAY_MS;
    let series = merge_into_skeleton(skel, vec![(today, 12), (yesterday, 5)]);
    let yesterday_sample = series
        .samples
        .iter()
        .find(|s| s.day_unix_ms == yesterday)
        .unwrap();
    assert_eq!(yesterday_sample.value, 5);
    let today_sample = series
        .samples
        .iter()
        .find(|s| s.day_unix_ms == today)
        .unwrap();
    assert_eq!(today_sample.value, 12);
}

#[test]
fn window_start_is_30_buckets_back() {
    let now = 1_700_000_000_000_i64;
    let start = window_start_ms(now);
    let today = day_floor_ms(now);
    assert_eq!(
        today - start,
        i64::from(DAILY_TIMESERIES_DAYS - 1) * ONE_DAY_MS
    );
}
