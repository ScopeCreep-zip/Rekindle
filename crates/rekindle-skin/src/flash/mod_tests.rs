use super::*;

#[test]
fn a_square_wave_counts_one_flash_per_cycle() {
    // 4 Hz black/white: 8 changes a second, 4 flashes.
    let series = [0.0, 1.0];
    let found = changes(&series, &[0.0, 120.0], 250.0, 6);
    assert_eq!(flashes_at(&found, 250.0), 4);
    // A 0.05 wobble is no change.
    let faint = changes(&[0.3, 0.35], &[0.0, 50.0], 100.0, 12);
    assert!(faint.is_empty());
    // Both ends above 0.80: no flash (the darker image is not below).
    let bright = changes(&[0.85, 1.0], &[0.0, 50.0], 100.0, 12);
    assert!(bright.is_empty());
}
