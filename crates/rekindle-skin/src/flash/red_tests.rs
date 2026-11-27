use super::*;

fn count(colours: &[[u8; 3]]) -> usize {
    let series: Vec<_> = colours.iter().map(|c| chroma(*c)).collect();
    let starts: Vec<_> = (0..u8::try_from(colours.len()).unwrap())
        .map(|i| f64::from(i) * 100.0)
        .collect();
    let period = starts.last().map_or(0.0, |s| s + 100.0);
    changes(&series, &starts, period, 4).len()
}

#[test]
fn saturated_red_against_black_white_and_grey_transitions() {
    // Four cycles, two transitions each, less the first state.
    assert_eq!(count(&[[255, 0, 0], [0, 0, 0]]), 7);
    assert_eq!(count(&[[255, 0, 0], [255, 255, 255]]), 7);
    assert_eq!(count(&[[200, 0, 0], [90, 90, 90]]), 7);
    // Linear redness: #ff3c3c is saturated (0.92), so it flashes too.
    assert_eq!(count(&[[255, 60, 60], [255, 255, 255]]), 7);
}

#[test]
fn red_to_red_and_unsaturated_pairs_do_not() {
    // The same chromaticity at two levels: a luminance flash only.
    assert_eq!(count(&[[255, 0, 0], [96, 0, 0]]), 0);
    // Green against blue: no state is saturated red.
    assert_eq!(count(&[[0, 255, 0], [0, 0, 255]]), 0);
    // Pink (redness 0.73 linear) against white: not saturated.
    assert_eq!(count(&[[255, 120, 120], [255, 255, 255]]), 0);
}

#[test]
fn the_thresholds_sit_where_the_note_puts_them() {
    assert!(chroma([255, 0, 0]).ratio >= SATURATED);
    assert!(chroma([255, 120, 120]).ratio < SATURATED);
    // sRGB red is 0.26 from D65 in u′v′: past 0.2.
    assert!(apart(chroma([255, 0, 0]), chroma([255, 255, 255])));
    assert!(!apart(chroma([255, 0, 0]), chroma([128, 0, 0])));
}
