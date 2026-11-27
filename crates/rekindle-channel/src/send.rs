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

use rekindle_codec::community::channel_record::ChannelMessage;

use crate::error::ChannelError;

pub use rekindle_secrets::channel_body::{
    decrypt_channel_body, encrypt_channel_body, BodyPosition,
};

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
    /// The message id of the message this one replies to, when it does.
    pub reply_to: Option<String>,
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
        reply_to,
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
        reply_to,
        revives: None,
        lamport_ts,
        message_id: Some(message_id),
        attachment: None,
        flags: mention_flag_bits,
        mentioned_pseudonyms,
        mentioned_roles,
    }
}

#[cfg(test)]
#[path = "send_tests.rs"]
mod tests;
