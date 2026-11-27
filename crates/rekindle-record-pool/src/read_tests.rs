use super::plan_reads;

#[test]
fn a_subkey_with_no_value_anywhere_is_not_read() {
    assert!(plan_reads([(4, None, None)]).is_empty());
}

#[test]
fn a_current_local_copy_is_read_locally() {
    assert_eq!(plan_reads([(4, Some(3), Some(3))]), vec![(4, false)]);
    // The network answered nothing newer (or nothing): ours stands.
    assert_eq!(plan_reads([(5, Some(3), None)]), vec![(5, false)]);
}

#[test]
fn only_a_newer_network_value_is_fetched_from_the_network() {
    assert_eq!(
        plan_reads([
            (1, Some(2), Some(7)),
            (2, None, Some(0)),
            (3, Some(9), Some(4))
        ]),
        vec![(1, true), (2, true), (3, false)]
    );
}
