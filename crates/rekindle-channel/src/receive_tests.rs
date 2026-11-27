use super::*;
use crate::send::{build_channel_message, BuildChannelMessageParams};

fn sample_message_with(flags: u32, pseudos: Vec<String>, roles: Vec<String>) -> ChannelMessage {
    build_channel_message(BuildChannelMessageParams {
        sequence: 1,
        sender_pseudonym: "sender".into(),
        ciphertext: vec![],
        mek_generation: 1,
        timestamp_ms: 0,
        lamport_ts: 0,
        message_id: "msg".into(),
        reply_to: None,
        mention_flag_bits: flags,
        mentioned_pseudonyms: pseudos,
        mentioned_roles: roles,
    })
}

#[test]
fn extract_mentions_detects_everyone() {
    let msg = sample_message_with(MENTION_EVERYONE, vec![], vec![]);
    let signals = extract_mention_signals(&msg, "me", &[]);
    assert!(signals.mention_everyone);
    assert!(!signals.mention_here);
    assert!(signals.warrants_notification());
}

#[test]
fn extract_mentions_detects_here() {
    let msg = sample_message_with(MENTION_HERE, vec![], vec![]);
    let signals = extract_mention_signals(&msg, "me", &[]);
    assert!(!signals.mention_everyone);
    assert!(signals.mention_here);
    assert!(signals.warrants_notification());
}

#[test]
fn extract_mentions_detects_direct_pseudonym() {
    let msg = sample_message_with(0, vec!["aBc123".into()], vec![]);
    let signals = extract_mention_signals(&msg, "abc123", &[]);
    assert!(signals.mentioned_local_pseudonym);
    assert!(signals.warrants_notification());
}

#[test]
fn extract_mentions_detects_local_role() {
    let msg = sample_message_with(0, vec![], vec!["role_a".into()]);
    let signals = extract_mention_signals(&msg, "me", &["role_b".into(), "role_a".into()]);
    assert!(signals.mentioned_local_role);
    assert!(signals.warrants_notification());
}

#[test]
fn extract_mentions_returns_no_signals_when_uninvolved() {
    let msg = sample_message_with(0, vec!["other".into()], vec!["role_x".into()]);
    let signals = extract_mention_signals(&msg, "me", &["role_a".into()]);
    assert!(!signals.warrants_notification());
    assert_eq!(signals, MentionSignals::default());
}
