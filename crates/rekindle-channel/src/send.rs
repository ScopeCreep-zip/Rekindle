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
//!
//! The body codec (AES-256-GCM with the record/subkey/Lamport AAD) is
//! `rekindle_secrets::channel_body`, shared by every host and re-exported
//! here.

use rekindle_codec::community::channel_record::{ChannelMessage, CHANNEL_OWNER_SUBKEY_COUNT};

use crate::error::ChannelError;

pub use rekindle_secrets::channel_body::{
    decrypt_channel_body, encrypt_channel_body, BodyPosition,
};

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

/// Already-computed inputs for the wire `ChannelMessage` constructor.
///
/// Owns its string/byte payloads because the orchestrator hands ownership
/// of freshly-built values (encrypted body, mention metadata, message id)
/// straight to the constructor — borrowing here would only force the
/// callers to keep originals alive across the move into `ChannelMessage`.
pub struct BuildChannelMessageParams {
    pub sequence: u64,
    pub sender_pseudonym: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp_ms: i64,
    pub lamport_ts: u64,
    pub message_id: String,
    pub mention_flag_bits: u32,
    pub mentioned_pseudonyms: Vec<String>,
    pub mentioned_roles: Vec<String>,
}

/// Pure constructor for the wire `ChannelMessage`. All inputs are
/// already-computed by the orchestrator (lamport_ts, sequence, sender
/// pseudonym, encrypted body, mention metadata). Returns the struct
/// ready for capnp encoding + DHT subkey write.
#[must_use]
pub fn build_channel_message(params: BuildChannelMessageParams) -> ChannelMessage {
    let BuildChannelMessageParams {
        sequence,
        sender_pseudonym,
        ciphertext,
        mek_generation,
        timestamp_ms,
        lamport_ts,
        message_id,
        mention_flag_bits,
        mentioned_pseudonyms,
        mentioned_roles,
    } = params;
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
        let msg = build_channel_message(BuildChannelMessageParams {
            sequence: 42,
            sender_pseudonym: "abc".into(),
            ciphertext: vec![1, 2, 3],
            mek_generation: 7,
            timestamp_ms: 1_000_000,
            lamport_ts: 99,
            message_id: "msg_1".into(),
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
}
