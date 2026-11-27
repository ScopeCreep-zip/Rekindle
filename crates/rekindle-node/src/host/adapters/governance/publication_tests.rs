use super::*;

#[test]
fn a_refusal_is_announced_and_a_landing_only_once_its_row_is_gone() {
    let before = vec![
        ("a".to_owned(), OpState::Applied),
        ("b".to_owned(), OpState::Applied),
        ("c".to_owned(), OpState::PendingDeps),
        ("d".to_owned(), OpState::PendingDeps),
    ];
    let after = vec![
        ("a".to_owned(), OpState::Applied),
        ("b".to_owned(), OpState::Quarantined),
        ("c".to_owned(), OpState::Applied),
        ("d".to_owned(), OpState::Applied),
        ("e".to_owned(), OpState::Equivocation),
    ];
    let announced = changed_verdicts(&before, &after, &|op| op == "d");
    let ops: Vec<&str> = announced.iter().map(|(op, _)| op.as_str()).collect();
    assert_eq!(ops, ["b", "c", "e"], "{announced:?}");
    assert!(matches!(announced[0].1, OpPublicationState::Failed(_)));
    assert_eq!(announced[1].1, OpPublicationState::Landed);
    assert!(matches!(announced[2].1, OpPublicationState::Failed(_)));
}
