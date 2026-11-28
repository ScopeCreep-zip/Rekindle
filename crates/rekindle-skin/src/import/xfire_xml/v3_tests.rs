use super::*;

#[test]
fn the_table_is_sorted() {
    assert!(NAMES.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(NAMES.len(), 68);
}
