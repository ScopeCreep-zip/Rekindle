//! DM wire envelope — serialized to JSON, written to DHT subkeys.
//!
//! Carries the ciphertext from either encryption path:
//! - 1:1 DMs: `encrypted_header` + `body` from Triple Ratchet
//! - Group DMs: `body` only from AES-256-GCM via MEK (`encrypted_header` empty)
//!
//! The receiver distinguishes the two paths via `DmSessionMeta.is_group`
//! loaded from the store BEFORE parsing the envelope. The envelope
//! carries no discriminant — adding one would be redundant wire bytes
//! and a desync attack vector.

use serde::{Deserialize, Serialize};

use crate::crypto::mek;
use crate::dm::error::DmError;

/// DM wire envelope as serialized to a DHT subkey value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmCiphertext {
    /// Triple Ratchet encrypted header (1:1 DMs).
    /// Empty for group DMs (MEK has no header).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub encrypted_header: Vec<u8>,

    /// Ciphertext — Triple Ratchet body (1:1) or AES-256-GCM via MEK (group).
    pub body: Vec<u8>,

    /// Sender-local sequence (incremented per write to our subkey).
    pub sequence: u64,

    /// Sender-side wall clock (ms since unix epoch).
    pub timestamp_ms: u64,

    /// Group DMs: generation of MEK used to encrypt `body`. The receiver
    /// materializes this generation from its `DmMekChain`.
    /// 1:1 DMs: ignored by receiver (ratchet state is implicit).
    pub mek_generation: u64,
}

// ── Build helpers ───────────────────────────────────────────────────

/// Build envelope for a 1:1 DM (Triple Ratchet output).
pub fn build_ratchet_envelope(
    encrypted_header: Vec<u8>,
    ciphertext: Vec<u8>,
    sequence: u64,
    timestamp_ms: u64,
) -> Result<Vec<u8>, DmError> {
    let envelope = DmCiphertext {
        encrypted_header,
        body: ciphertext,
        sequence,
        timestamp_ms,
        mek_generation: 0,
    };
    serde_json::to_vec(&envelope)
        .map_err(|e| DmError::EncryptFailed(format!("ratchet envelope serialize: {e}")))
}

/// Build envelope for a group DM (MEK AES-256-GCM).
pub fn build_mek_envelope(
    mek_key: [u8; 32],
    mek_generation: u64,
    body: &str,
    sequence: u64,
    timestamp_ms: u64,
) -> Result<Vec<u8>, DmError> {
    let ciphertext = mek::mek_encrypt(&mek_key, body.as_bytes(), &[])
        .map_err(|e| DmError::EncryptFailed(e.to_string()))?;
    let envelope = DmCiphertext {
        encrypted_header: Vec::new(),
        body: ciphertext,
        sequence,
        timestamp_ms,
        mek_generation,
    };
    serde_json::to_vec(&envelope)
        .map_err(|e| DmError::EncryptFailed(format!("mek envelope serialize: {e}")))
}

// ── Parse + decrypt helpers ─────────────────────────────────────────

/// Parse a serialized envelope from raw DHT subkey bytes.
pub fn parse_envelope(raw_value: &[u8]) -> Result<DmCiphertext, DmError> {
    serde_json::from_slice(raw_value)
        .map_err(|e| DmError::EnvelopeDecode(format!("dm envelope: {e}")))
}

/// Decrypt the body of a group DM envelope using MEK bytes.
pub fn decrypt_mek_body(envelope: &DmCiphertext, mek_key: [u8; 32]) -> Result<String, DmError> {
    let plaintext = mek::mek_decrypt(&mek_key, &envelope.body, &[])
        .map_err(|e| DmError::DecryptFailed(e.to_string()))?;
    String::from_utf8(plaintext)
        .map_err(|e| DmError::DecryptFailed(format!("dm body not utf-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mek_round_trip() {
        let mek_key = [0xab; 32];
        let wire = build_mek_envelope(mek_key, 3, "hello world", 1, 1000).unwrap();
        let envelope = parse_envelope(&wire).unwrap();
        assert_eq!(envelope.sequence, 1);
        assert_eq!(envelope.timestamp_ms, 1000);
        assert_eq!(envelope.mek_generation, 3);
        assert!(envelope.encrypted_header.is_empty());
        let body = decrypt_mek_body(&envelope, mek_key).unwrap();
        assert_eq!(body, "hello world");
    }

    #[test]
    fn ratchet_envelope_carries_header() {
        let header = vec![1, 2, 3, 4];
        let ct = vec![5, 6, 7, 8];
        let wire = build_ratchet_envelope(header.clone(), ct.clone(), 42, 9999).unwrap();
        let envelope = parse_envelope(&wire).unwrap();
        assert_eq!(envelope.encrypted_header, header);
        assert_eq!(envelope.body, ct);
        assert_eq!(envelope.sequence, 42);
        assert_eq!(envelope.mek_generation, 0);
    }

    #[test]
    fn empty_header_skipped_in_json() {
        let wire = build_mek_envelope([0xab; 32], 0, "x", 1, 1).unwrap();
        let json = String::from_utf8(wire).unwrap();
        assert!(!json.contains("encrypted_header"), "empty header must be skipped");
    }

    #[test]
    fn wrong_mek_fails() {
        let wire = build_mek_envelope([0xab; 32], 0, "secret", 1, 1).unwrap();
        let envelope = parse_envelope(&wire).unwrap();
        let err = decrypt_mek_body(&envelope, [0xcd; 32]).unwrap_err();
        assert!(matches!(err, DmError::DecryptFailed(_)));
    }

    #[test]
    fn malformed_envelope_fails() {
        let err = parse_envelope(b"not json").unwrap_err();
        assert!(matches!(err, DmError::EnvelopeDecode(_)));
    }
}
