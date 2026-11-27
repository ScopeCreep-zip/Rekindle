use rekindle_types::analytics::ActivityByHour;

#[test]
fn peak_hour_picks_largest_bucket() {
    let mut h = ActivityByHour::default();
    h.hour_counts[14] = 100;
    h.hour_counts[3] = 5;
    h.hour_counts[20] = 50;
    assert_eq!(h.peak_hour(), 14);
}

#[test]
fn peak_hour_returns_zero_for_empty_histogram() {
    let h = ActivityByHour::default();
    assert_eq!(h.peak_hour(), 0);
}

#[test]
fn peak_hour_breaks_ties_by_lowest_hour() {
    let mut h = ActivityByHour::default();
    h.hour_counts[5] = 10;
    h.hour_counts[10] = 10;
    // The first match in the iteration wins ties.
    assert_eq!(h.peak_hour(), 5);
}
