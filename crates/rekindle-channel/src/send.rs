//! Phase 19.c — pure channel-send protocol primitives.
//!
//! Ported from src-tauri/services/community/channel_messages.rs.
//! Chiral split (matches Phase 17/18 pattern): the src-tauri orchestrator
//! keeps full `send_message` (AppState mutations, DB writes, DHT writes,
//! retry queue, mention resolution); this module hosts the PURE pieces
//! that can be tested without a runtime:
//!
//! - `slowmode_check` — pure decision: is the next send allowed?
//! - `build_channel_message` — pure constructor for the wire shape
//! - `encrypt_channel_body` — symmetric encrypt with AAD binding

use rekindle_crypto::group::media_key::{ChannelAad, MediaEncryptionKey as CryptoMek};
use rekindle_protocol::dht::community::channel_record::{
    ChannelMessage, CHANNEL_OWNER_SUBKEY_COUNT,
};

use crate::deps::ChannelMek;
use crate::error::ChannelError;

/// Architecture §28.4 — channel SMPL subkey index for a member writing
/// their slot's stream of messages. Pure offset from `CHANNEL_OWNER_SUBKEY_COUNT`.
#[must_use]
pub fn channel_message_subkey(member_index: u32) -> u32 {
    u32::from(CHANNEL_OWNER_SUBKEY_COUNT) + member_index
}

/// Architecture §28.7 — slowmode gate. Returns `Ok(())` when the send
/// is allowed; returns `Err(SlowmodeActive)` with milliseconds-to-wait
/// when not. Bypass is the caller's responsibility (pass `Ok(())` from
/// the bypass branch without invoking this helper).
pub fn slowmode_check(
    slowmode_seconds: Option<u32>,
    last_send_ms: u64,
    now_ms: u64,
) -> Result<(), ChannelError> {
    let Some(secs) = slowmode_seconds.filter(|&s| s > 0) else {
        return Ok(());
    };
    let elapsed_ms = now_ms.saturating_sub(last_send_ms);
    let required_ms = u64::from(secs).saturating_mul(1000);
    if elapsed_ms < required_ms {
        return Err(ChannelError::SlowmodeActive {
            wait_ms: required_ms - elapsed_ms,
        });
    }
    Ok(())
}

/// Architecture §8 line 1626 — symmetric encrypt the plaintext body
/// under the per-channel MEK with AAD binding `(channel_record_key,
/// subkey_index, lamport_ts)` so the ciphertext cannot be replayed to
/// a different channel/slot/sequence.
pub fn encrypt_channel_body(
    mek: &ChannelMek,
    channel_record_key: &str,
    subkey_index: u32,
    lamport_ts: u64,
    body: &[u8],
) -> Result<Vec<u8>, ChannelError> {
    let crypto_mek = CryptoMek::from_bytes(mek.key_bytes, mek.generation);
    let aad = ChannelAad {
        channel_record_key: channel_record_key.as_bytes(),
        subkey_index,
        lamport_ts,
    };
    crypto_mek
        .encrypt_with_aad(body, aad)
        .map_err(|e| ChannelError::Encrypt(format!("MEK encryption failed: {e}")))
}

/// Pure constructor for the wire `ChannelMessage`. All inputs are
/// already-computed by the orchestrator (lamport_ts, sequence, sender
/// pseudonym, encrypted body, mention metadata). Returns the struct
/// ready for capnp encoding + DHT subkey write.
#[allow(clippy::too_many_arguments, reason = "Mirrors wire-shape constructor; passing a context struct would just re-shape the args without semantic clarity.")]
pub fn build_channel_message(
    sequence: u64,
    sender_pseudonym: String,
    ciphertext: Vec<u8>,
    mek_generation: u64,
    timestamp_ms: i64,
    lamport_ts: u64,
    message_id: String,
    mention_flag_bits: u32,
    mentioned_pseudonyms: Vec<String>,
    mentioned_roles: Vec<String>,
) -> ChannelMessage {
    ChannelMessage {
        sequence,
        sender_pseudonym,
        ciphertext,
        mek_generation,
        timestamp: u64::try_from(timestamp_ms).unwrap_or_default(),
        reply_to: None,
        lamport_ts,
        message_id: Some(message_id),
        attachment: None,
        flags: mention_flag_bits,
        mentioned_pseudonyms,
        mentioned_roles,
    }
}

#[cfg(test)]
mod tests {
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
        let msg = build_channel_message(
            42,
            "abc".into(),
            vec![1, 2, 3],
            7,
            1_000_000,
            99,
            "msg_1".into(),
            0b11,
            vec!["pseu1".into()],
            vec!["role1".into()],
        );
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

    #[test]
    fn encrypt_channel_body_roundtrip_through_decrypt() {
        let mek = ChannelMek {
            generation: 1,
            key_bytes: [42u8; 32],
        };
        let ct = encrypt_channel_body(&mek, "rkey", 7, 99, b"hello").expect("encrypt");
        // Decrypt with matching CryptoMek + AAD must round-trip.
        let crypto = CryptoMek::from_bytes(mek.key_bytes, mek.generation);
        let aad = ChannelAad {
            channel_record_key: b"rkey",
            subkey_index: 7,
            lamport_ts: 99,
        };
        let pt = crypto.decrypt_with_aad(&ct, aad).expect("decrypt");
        assert_eq!(pt, b"hello");
    }

    #[test]
    fn encrypt_channel_body_aad_binding_rejects_wrong_subkey() {
        let mek = ChannelMek {
            generation: 1,
            key_bytes: [42u8; 32],
        };
        let ct = encrypt_channel_body(&mek, "rkey", 7, 99, b"hello").expect("encrypt");
        let crypto = CryptoMek::from_bytes(mek.key_bytes, mek.generation);
        let wrong_aad = ChannelAad {
            channel_record_key: b"rkey",
            subkey_index: 8, // wrong subkey
            lamport_ts: 99,
        };
        assert!(crypto.decrypt_with_aad(&ct, wrong_aad).is_err());
    }
}
