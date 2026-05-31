//! Phase 19.d — pure channel-receive protocol primitives.
//!
//! Ported from src-tauri/services/community/message_notifications.rs
//! decrypt + mention-signal paths. Chiral split (matches Phase 17/18
//! and Phase 19.c send.rs): pure decrypt + parse + reader-validate +
//! mention-signal extraction live here; src-tauri retains the
//! orchestrator that gathers MEKs, looks up state, and emits events.

use rekindle_crypto::group::media_key::{ChannelAad, MediaEncryptionKey as CryptoMek};
use rekindle_protocol::dht::community::channel_record::ChannelMessage;
use rekindle_types::channel::flags::{MENTION_EVERYONE, MENTION_HERE};

use crate::deps::ChannelMek;
use crate::error::ChannelError;

/// Symmetric decrypt the ciphertext under the given MEK with AAD
/// validation `(channel_record_key, subkey_index, lamport_ts)`. The AAD
/// must match the one the sender used (architecture §8 line 1626);
/// mismatch triggers a `Decrypt` error.
pub fn decrypt_channel_body(
    mek: &ChannelMek,
    channel_record_key: &str,
    subkey_index: u32,
    lamport_ts: u64,
    ciphertext: &[u8],
) -> Result<Vec<u8>, ChannelError> {
    let crypto_mek = CryptoMek::from_bytes(mek.key_bytes, mek.generation);
    let aad = ChannelAad {
        channel_record_key: channel_record_key.as_bytes(),
        subkey_index,
        lamport_ts,
    };
    crypto_mek
        .decrypt_with_aad(ciphertext, aad)
        .map_err(|e| ChannelError::Decrypt(format!("MEK AAD decrypt failed: {e}")))
}

/// Legacy-compatible decrypt: try AAD-bound decrypt first; on failure
/// fall back to the no-AAD path that pre-AAD messages used. Matches
/// the src-tauri `message_notifications` decrypt waterfall.
pub fn decrypt_channel_body_with_legacy_fallback(
    mek: &ChannelMek,
    channel_record_key: Option<&str>,
    subkey_index: u32,
    lamport_ts: u64,
    ciphertext: &[u8],
) -> Result<Vec<u8>, ChannelError> {
    let crypto_mek = CryptoMek::from_bytes(mek.key_bytes, mek.generation);
    if let Some(record_key) = channel_record_key {
        let aad = ChannelAad {
            channel_record_key: record_key.as_bytes(),
            subkey_index,
            lamport_ts,
        };
        if let Ok(pt) = crypto_mek.decrypt_with_aad(ciphertext, aad) {
            return Ok(pt);
        }
    }
    crypto_mek
        .decrypt(ciphertext)
        .map_err(|e| ChannelError::Decrypt(format!("legacy MEK decrypt failed: {e}")))
}

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
    use crate::send::{build_channel_message, encrypt_channel_body};

    fn sample_mek() -> ChannelMek {
        ChannelMek {
            generation: 1,
            key_bytes: [42u8; 32],
        }
    }

    #[test]
    fn decrypt_roundtrip_through_encrypt() {
        let mek = sample_mek();
        let ct = encrypt_channel_body(&mek, "rkey", 7, 99, b"hello").expect("encrypt");
        let pt = decrypt_channel_body(&mek, "rkey", 7, 99, &ct).expect("decrypt");
        assert_eq!(pt, b"hello");
    }

    #[test]
    fn decrypt_rejects_wrong_aad() {
        let mek = sample_mek();
        let ct = encrypt_channel_body(&mek, "rkey", 7, 99, b"hello").expect("encrypt");
        assert!(decrypt_channel_body(&mek, "rkey", 8, 99, &ct).is_err());
        assert!(decrypt_channel_body(&mek, "wrong", 7, 99, &ct).is_err());
        assert!(decrypt_channel_body(&mek, "rkey", 7, 100, &ct).is_err());
    }

    #[test]
    fn legacy_fallback_when_no_record_key_supplied() {
        let mek = sample_mek();
        let crypto_mek = CryptoMek::from_bytes(mek.key_bytes, mek.generation);
        // Encrypt WITHOUT AAD (legacy path).
        let ct = crypto_mek.encrypt(b"legacy body").expect("encrypt");
        let pt = decrypt_channel_body_with_legacy_fallback(&mek, None, 0, 0, &ct).expect("decrypt");
        assert_eq!(pt, b"legacy body");
    }

    #[test]
    fn legacy_fallback_prefers_aad_path_when_available() {
        let mek = sample_mek();
        let ct = encrypt_channel_body(&mek, "rkey", 7, 99, b"new body").expect("encrypt");
        let pt = decrypt_channel_body_with_legacy_fallback(&mek, Some("rkey"), 7, 99, &ct)
            .expect("decrypt");
        assert_eq!(pt, b"new body");
    }

    fn sample_message_with(flags: u32, pseudos: Vec<String>, roles: Vec<String>) -> ChannelMessage {
        build_channel_message(
            1,
            "sender".into(),
            vec![],
            1,
            0,
            0,
            "msg".into(),
            flags,
            pseudos,
            roles,
        )
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
