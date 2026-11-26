use super::fanout_degree;

#[test]
fn fanout_degree_matches_architecture_ranges() {
    assert_eq!(fanout_degree(0), 0);
    assert_eq!(fanout_degree(1), 1);
    assert_eq!(fanout_degree(5), 5);
    assert_eq!(fanout_degree(6), 6);
    assert_eq!(fanout_degree(7), 6);
    assert_eq!(fanout_degree(20), 6);
    assert_eq!(fanout_degree(21), 6);
    assert_eq!(fanout_degree(60), 6);
    assert_eq!(fanout_degree(61), 8);
}
