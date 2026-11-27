use super::*;

#[test]
fn table_is_sorted_and_counts_hold() {
    assert!(NAMES.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(NAMES.len(), 63);
    let dropped = NAMES
        .iter()
        .filter(|(_, t)| matches!(t, Target::Dropped(_)))
        .count();
    assert_eq!(dropped, 9);
}
