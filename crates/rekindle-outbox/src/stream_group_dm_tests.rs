use super::{OutboxRetry, StreamKind};

/// A group send settles on its chain write: no ack, retryable, backed off.
#[test]
fn a_group_dm_row_awaits_no_ack_and_may_be_retried() {
    let kind = StreamKind::GroupDm;
    assert_eq!(StreamKind::parse(kind.as_str()), Some(kind));
    assert_eq!(kind.stream("ab"), "group_dm:ab");
    assert!(!kind.awaits_ack());
    assert!(kind.user_retryable());
    assert!(matches!(kind.policy(), OutboxRetry::Backoff { .. }));
}

/// A key transfer settles on its receipt: no ack row, retryable,
/// backed off like a handshake frame.
#[test]
fn a_mek_transfer_row_settles_on_its_receipt_and_backs_off_like_a_handshake() {
    let kind = StreamKind::MekTransfer;
    assert_eq!(StreamKind::parse("mek_transfer"), Some(kind));
    assert_eq!(kind.stream("c:p"), "mek_transfer:c:p");
    assert!(!kind.awaits_ack());
    assert_eq!(kind.first_reask(&rekindle_utils::OsRandom), None);
    assert!(kind.user_retryable());
    assert_eq!(kind.policy(), StreamKind::FriendHandshake.policy());
}

/// An op append settles on its log write, like a group send.
#[test]
fn a_governance_op_row_awaits_no_ack_and_may_be_retried() {
    let kind = StreamKind::GovernanceOp;
    assert_eq!(StreamKind::parse(kind.as_str()), Some(kind));
    assert_eq!(kind.stream("c"), "governance_op:c");
    assert!(!kind.awaits_ack());
    assert!(kind.user_retryable());
    assert!(matches!(kind.policy(), OutboxRetry::Backoff { .. }));
}
