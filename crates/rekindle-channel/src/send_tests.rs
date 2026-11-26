use super::*;

#[test]
fn slowmode_check_passes_when_no_slowmode_configured() {
    assert!(slowmode_check(None, 0, 1000).is_ok());
    assert!(slowmode_check(Some(0), 0, 1000).is_ok());
}

#[test]
fn slowmode_check_passes_when_window_elapsed() {
    assert!(slowmode_check(Some(5), 1000, 6001).is_ok());
    assert!(slowmode_check(Some(5), 1000, 6000).is_ok());
}

#[test]
fn slowmode_check_rejects_within_window_with_wait_ms() {
    let err = slowmode_check(Some(5), 1000, 3000).expect_err("within window");
    match err {
        ChannelError::SlowmodeActive { wait_ms } => assert_eq!(wait_ms, 3000),
        other => panic!("expected SlowmodeActive, got {other:?}"),
    }
}

#[test]
fn slowmode_check_saturating_arithmetic_doesnt_panic() {
    // Last send "in the future" (clock skew) — elapsed = 0, must reject.
    assert!(slowmode_check(Some(5), 10_000, 1_000).is_err());
}

#[test]
fn build_channel_message_carries_all_inputs() {
    let msg = build_channel_message(BuildChannelMessageParams {
        sequence: 42,
        sender_pseudonym: "abc".into(),
        ciphertext: vec![1, 2, 3],
        mek_generation: 7,
        timestamp_ms: 1_000_000,
        lamport_ts: 99,
        message_id: "msg_1".into(),
        reply_to: None,
        mention_flag_bits: 0b11,
        mentioned_pseudonyms: vec!["pseu1".into()],
        mentioned_roles: vec!["role1".into()],
    });
    assert_eq!(msg.sequence, 42);
    assert_eq!(msg.sender_pseudonym, "abc");
    assert_eq!(msg.ciphertext, vec![1, 2, 3]);
    assert_eq!(msg.mek_generation, 7);
    assert_eq!(msg.timestamp, 1_000_000);
    assert_eq!(msg.lamport_ts, 99);
    assert_eq!(msg.message_id.as_deref(), Some("msg_1"));
    assert_eq!(msg.flags, 0b11);
    assert!(msg.reply_to.is_none());
    assert!(msg.attachment.is_none());
}
