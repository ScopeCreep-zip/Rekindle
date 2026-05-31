//! Phase 13 — DM wire envelope (`DmCiphertext`) + encrypt/decrypt
//! helpers.
//!
//! DM bodies travel as JSON envelopes over Veilid DHT subkey writes:
//! `{ body: <AES-GCM ciphertext>, sequence: u64, timestamp_ms: u64,
//!    mek_generation: u64 }`. The MEK generation in the envelope lets
//! the receiver pick the right historical key (architecture §5.2:1100).
//!
//! Pure logic — no DHT, no DB, no AppState. The src-tauri shell handles
//! the actual subkey write + persistence; this module owns the wire
//! format and the AES-GCM bridge to `rekindle-crypto::MediaEncryptionKey`.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use serde::{Deserialize, Serialize};

use crate::error::DmError;

/// Forward-secure ratchet trigger thresholds (architecture §27.1).
/// Either trigger fires the next sender-side ratchet advance.
pub const DM_RATCHET_MESSAGE_INTERVAL: u64 = 100;
pub const DM_RATCHET_TIME_INTERVAL_SECS: i64 = 86_400; // 24 h

/// DM wire envelope as serialized to a DHT subkey value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmCiphertext {
    /// AES-GCM-encoded body.
    pub body: Vec<u8>,
    /// Sender-local sequence (incremented per write to our subkey).
    pub sequence: u64,
    /// Sender-side wall clock (ms since unix epoch).
    pub timestamp_ms: u64,
    /// Generation of the MEK used to encrypt `body`. The receiver picks
    /// the matching historical key from its `DmMekChain`.
    pub mek_generation: u64,
}

/// Build a wire envelope: encrypt `body` with the supplied MEK at the
/// given generation, package with `sequence` + `timestamp_ms`, and
/// serialize to JSON bytes ready to write to a DHT subkey.
pub fn build_envelope(
    mek_bytes: [u8; 32],
    mek_generation: u64,
    body: &str,
    sequence: u64,
    timestamp_ms: u64,
) -> Result<Vec<u8>, DmError> {
    let mek = MediaEncryptionKey::from_bytes(mek_bytes, mek_generation);
    let ciphertext = mek
        .encrypt(body.as_bytes())
        .map_err(|e| DmError::EncryptFailed(e.to_string()))?;
    let envelope = DmCiphertext {
        body: ciphertext,
        sequence,
        timestamp_ms,
        mek_generation,
    };
    serde_json::to_vec(&envelope).map_err(|e| DmError::Serialize(format!("dm envelope: {e}")))
}

/// Parse a serialized envelope. Callers typically then look up the
/// MEK for `envelope.mek_generation` (via `DmMekChain::for_generation`)
/// and pass both to [`decrypt_body`].
pub fn parse_envelope(raw_value: &[u8]) -> Result<DmCiphertext, DmError> {
    serde_json::from_slice(raw_value)
        .map_err(|e| DmError::EnvelopeDecode(format!("dm envelope: {e}")))
}

/// Decrypt the `body` field of a parsed envelope using the MEK bytes
/// for its generation. Returns the plaintext as a UTF-8 string;
/// non-UTF-8 plaintext is a decrypt failure (DMs are text-only).
pub fn decrypt_body(envelope: &DmCiphertext, mek_bytes: [u8; 32]) -> Result<String, DmError> {
    let mek = MediaEncryptionKey::from_bytes(mek_bytes, envelope.mek_generation);
    let plaintext_bytes = mek
        .decrypt(&envelope.body)
        .map_err(|e| DmError::DecryptFailed(e.to_string()))?;
    String::from_utf8(plaintext_bytes)
        .map_err(|e| DmError::DecryptFailed(format!("dm body not utf-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_mek() -> [u8; 32] {
        [0xab; 32]
    }

    #[test]
    fn round_trip_text_body() {
        let env_bytes = build_envelope(sample_mek(), 0, "hello world", 1, 1_000).unwrap();
        let envelope = parse_envelope(&env_bytes).unwrap();
        assert_eq!(envelope.sequence, 1);
        assert_eq!(envelope.timestamp_ms, 1_000);
        assert_eq!(envelope.mek_generation, 0);
        let body = decrypt_body(&envelope, sample_mek()).unwrap();
        assert_eq!(body, "hello world");
    }

    #[test]
    fn round_trip_unicode() {
        let body = "héllo 世界 🌍";
        let env_bytes = build_envelope(sample_mek(), 7, body, 42, 99_999).unwrap();
        let envelope = parse_envelope(&env_bytes).unwrap();
        assert_eq!(envelope.mek_generation, 7);
        assert_eq!(decrypt_body(&envelope, sample_mek()).unwrap(), body);
    }

    #[test]
    fn empty_body_round_trips() {
        let env_bytes = build_envelope(sample_mek(), 0, "", 1, 1).unwrap();
        let envelope = parse_envelope(&env_bytes).unwrap();
        let body = decrypt_body(&envelope, sample_mek()).unwrap();
        assert_eq!(body, "");
    }

    #[test]
    fn wrong_key_fails_decrypt() {
        let env_bytes = build_envelope(sample_mek(), 0, "secret", 1, 1).unwrap();
        let envelope = parse_envelope(&env_bytes).unwrap();
        let wrong_key = [0xcd; 32];
        let err = decrypt_body(&envelope, wrong_key).unwrap_err();
        assert!(matches!(err, DmError::DecryptFailed(_)));
    }

    #[test]
    fn malformed_envelope_fails_parse() {
        let err = parse_envelope(b"not json at all").unwrap_err();
        assert!(matches!(err, DmError::EnvelopeDecode(_)));
    }

    #[test]
    fn envelope_carries_mek_generation_for_historical_lookup() {
        // Receiver sees the generation field BEFORE it picks the MEK.
        let env_bytes = build_envelope(sample_mek(), 17, "msg", 5, 100).unwrap();
        let envelope = parse_envelope(&env_bytes).unwrap();
        assert_eq!(envelope.mek_generation, 17);
        // Even before decrypt, the caller can drive chain.observed(17)
        // and chain.for_generation(17) — independent operations.
    }

    #[test]
    fn ratchet_thresholds_match_architecture_spec() {
        // Architecture §27.1 — 100 messages or 24 h, whichever first.
        assert_eq!(DM_RATCHET_MESSAGE_INTERVAL, 100);
        assert_eq!(DM_RATCHET_TIME_INTERVAL_SECS, 86_400);
    }
}
