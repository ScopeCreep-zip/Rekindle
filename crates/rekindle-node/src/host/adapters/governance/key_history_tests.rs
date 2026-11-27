use super::generations_owed;

#[test]
fn a_role_that_reads_history_is_owed_every_generation_oldest_first() {
    assert_eq!(generations_owed(3, true), vec![1, 2, 3]);
}

#[test]
fn a_role_without_history_is_owed_only_the_current_generation() {
    assert_eq!(generations_owed(3, false), vec![3]);
    assert_eq!(generations_owed(1, false), vec![1]);
}
