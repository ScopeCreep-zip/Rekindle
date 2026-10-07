//! Phase 19.d — pure channel-receive protocol primitives: mention-signal
//! extraction. The body codec is `rekindle_secrets::channel_body`
//! (re-exported from `send`).

use rekindle_protocol::dht::community::channel_record::ChannelMessage;
use rekindle_types::channel::flags::{MENTION_EVERYONE, MENTION_HERE};

/// Decoded mention signals from a `ChannelMessage.flags + mentioned_*`
/// payload. The receiver routes notifications based on these without
/// decrypting the body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentionSignals {
    /// The local member is directly @mentioned.
    pub mentioned_local_pseudonym: bool,
    /// The message contains `@everyone`.
    pub mention_everyone: bool,
    /// The message contains `@here` (online members only).
    pub mention_here: bool,
    /// One or more of the local member's role IDs is @-mentioned.
    pub mentioned_local_role: bool,
}

impl MentionSignals {
    /// Aggregate "should I notify?" decision. Adapter combines with
    /// per-channel notification level + DND rules before actually
    /// emitting NotificationEvent.
    #[must_use]
    pub fn warrants_notification(&self) -> bool {
        self.mentioned_local_pseudonym
            || self.mention_everyone
            || self.mention_here
            || self.mentioned_local_role
    }
}

/// Extract mention signals from a wire `ChannelMessage` given the
/// receiver's identity. Pure decode of the `flags` u32 + the
/// `mentioned_pseudonyms` / `mentioned_roles` Vec<String>s.
///
/// `my_pseudonym_hex` and `my_role_ids_hex` (as 32-byte hex strings)
/// are how the receiver matches their own identity against the
/// cleartext mention payload (architecture §28.5 — pseudonyms and
/// role IDs are sent in cleartext alongside ciphertext bodies so
/// notification routing skips decryption).
pub fn extract_mention_signals(
    message: &ChannelMessage,
    my_pseudonym_hex: &str,
    my_role_ids_hex: &[String],
) -> MentionSignals {
    let mention_everyone = (message.flags & MENTION_EVERYONE) == MENTION_EVERYONE;
    let mention_here = (message.flags & MENTION_HERE) == MENTION_HERE;
    let mentioned_local_pseudonym = message
        .mentioned_pseudonyms
        .iter()
        .any(|p| p.eq_ignore_ascii_case(my_pseudonym_hex));
    let mentioned_local_role = message.mentioned_roles.iter().any(|role_hex| {
        my_role_ids_hex
            .iter()
            .any(|mine| mine.eq_ignore_ascii_case(role_hex))
    });
    MentionSignals {
        mentioned_local_pseudonym,
        mention_everyone,
        mention_here,
        mentioned_local_role,
    }
}

#[cfg(test)]
mod tests {
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
}
