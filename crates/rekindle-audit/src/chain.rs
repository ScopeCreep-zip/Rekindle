//! BLAKE3 keyed-MAC hash chain.
//!
//! `AuditChain::append` extends the chain with one record, returning the
//! new entry. `AuditChain::verify` re-derives every MAC and asserts
//! contiguity. The genesis entry's `prev_mac` is the all-zero 32-byte
//! string; the first cursor is 1.

use blake3::{Hasher, KEY_LEN};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Length of a chain MAC in bytes (BLAKE3-keyed → 32 bytes).
pub const MAC_LEN: usize = 32;

/// Kinds of audit-worthy actions. Adding a variant is wire-format
/// safe because [`AuditRecord`] serializes via `serde_json` keyed on
/// variant names; old entries deserialize on unchanged variants.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditKind {
    FriendAdded,
    FriendRemoved,
    ChannelJoined,
    ChannelLeft,
    IdentityRotated,
    VaultUnlocked,
}

/// Logical contents of one audit event. The `payload_json` field of an
/// [`AuditEntry`] is `serde_json::to_vec(record)?`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditRecord {
    pub at_ms: i64,
    pub actor_pub: String,
    pub kind: AuditKind,
    pub payload: serde_json::Value,
}

/// One entry in the persisted chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub cursor: u64,
    pub prev_mac: [u8; MAC_LEN],
    pub mac: [u8; MAC_LEN],
    pub record: AuditRecord,
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error(
        "entry at cursor {cursor} (index {index}): prev_mac does not match preceding entry's mac"
    )]
    PrevMacMismatch { cursor: u64, index: usize },
    #[error("entry at cursor {cursor} (index {index}): recomputed MAC does not match stored mac (tampered payload)")]
    MacMismatch { cursor: u64, index: usize },
    #[error("entry at cursor {cursor} (index {index}): cursor is not monotonic (expected > {expected_after})")]
    NonMonotonicCursor {
        cursor: u64,
        index: usize,
        expected_after: u64,
    },
    #[error("payload serialization failed during verify: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Hash-chain producer/verifier. Keep one instance per vault session;
/// `append` is the only mutator.
pub struct AuditChain {
    key: Zeroizing<[u8; KEY_LEN]>,
    last_mac: [u8; MAC_LEN],
    last_cursor: u64,
}

impl AuditChain {
    /// Open at a known tail. Genesis is `last_mac = [0; 32]`, `last_cursor = 0`.
    #[must_use]
    pub fn open(key: Zeroizing<[u8; KEY_LEN]>, last_mac: [u8; MAC_LEN], last_cursor: u64) -> Self {
        Self {
            key,
            last_mac,
            last_cursor,
        }
    }

    /// Most recent MAC the chain has issued (or genesis if none yet).
    #[must_use]
    pub fn last_mac(&self) -> [u8; MAC_LEN] {
        self.last_mac
    }

    /// Most recent cursor the chain has issued (or 0 if none yet).
    #[must_use]
    pub fn last_cursor(&self) -> u64 {
        self.last_cursor
    }

    /// Extend the chain by one entry. Returns the new [`AuditEntry`].
    /// The caller is responsible for persisting it durably (e.g. SQLite).
    ///
    /// # Errors
    /// Returns `serde_json::Error` if the record cannot be serialized.
    pub fn append(&mut self, record: AuditRecord) -> Result<AuditEntry, serde_json::Error> {
        let cursor = self.last_cursor.saturating_add(1);
        let payload = serde_json::to_vec(&record)?;
        let mac = compute_mac(&self.key, &self.last_mac, cursor, &payload);
        let entry = AuditEntry {
            cursor,
            prev_mac: self.last_mac,
            mac,
            record,
        };
        self.last_mac = mac;
        self.last_cursor = cursor;
        Ok(entry)
    }

    /// Re-derive every MAC in `entries` and assert the chain links cleanly
    /// from genesis (or from the supplied starting `prev_mac`).
    ///
    /// `entries` must be sorted ascending by cursor and contiguous from
    /// the supplied genesis. The verifier does NOT trust `entry.prev_mac`
    /// blindly — it recomputes from the previous entry's `mac`.
    ///
    /// # Errors
    /// Returns a [`VerifyError`] identifying the first broken position.
    pub fn verify(&self, entries: &[AuditEntry]) -> Result<(), VerifyError> {
        let mut prev_mac = [0u8; MAC_LEN]; // genesis
        let mut prev_cursor = 0u64;
        for (index, entry) in entries.iter().enumerate() {
            if entry.cursor <= prev_cursor {
                return Err(VerifyError::NonMonotonicCursor {
                    cursor: entry.cursor,
                    index,
                    expected_after: prev_cursor,
                });
            }
            if entry.prev_mac != prev_mac {
                return Err(VerifyError::PrevMacMismatch {
                    cursor: entry.cursor,
                    index,
                });
            }
            let payload = serde_json::to_vec(&entry.record)?;
            let computed = compute_mac(&self.key, &prev_mac, entry.cursor, &payload);
            if computed != entry.mac {
                return Err(VerifyError::MacMismatch {
                    cursor: entry.cursor,
                    index,
                });
            }
            prev_mac = entry.mac;
            prev_cursor = entry.cursor;
        }
        Ok(())
    }
}

fn compute_mac(
    key: &[u8; KEY_LEN],
    prev_mac: &[u8; MAC_LEN],
    cursor: u64,
    payload: &[u8],
) -> [u8; MAC_LEN] {
    let mut h = Hasher::new_keyed(key);
    h.update(prev_mac);
    h.update(&cursor.to_le_bytes());
    h.update(payload);
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_key() -> Zeroizing<[u8; KEY_LEN]> {
        Zeroizing::new([7u8; KEY_LEN])
    }

    fn rec(kind: AuditKind, n: u64) -> AuditRecord {
        AuditRecord {
            at_ms: 1_700_000_000_000 + i64::try_from(n).unwrap(),
            actor_pub: format!("actor-{n}"),
            kind,
            payload: json!({"seq": n}),
        }
    }

    #[test]
    fn append_then_verify_roundtrip() {
        let key = fixture_key();
        let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
        let e1 = chain.append(rec(AuditKind::VaultUnlocked, 1)).unwrap();
        let e2 = chain.append(rec(AuditKind::FriendAdded, 2)).unwrap();
        let e3 = chain.append(rec(AuditKind::IdentityRotated, 3)).unwrap();
        assert_eq!(e1.cursor, 1);
        assert_eq!(e2.cursor, 2);
        assert_eq!(e3.cursor, 3);
        assert_eq!(e2.prev_mac, e1.mac);
        assert_eq!(e3.prev_mac, e2.mac);
        let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
        verifier
            .verify(&[e1, e2, e3])
            .expect("clean chain verifies");
    }

    #[test]
    fn detects_tampered_payload() {
        let key = fixture_key();
        let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
        let mut entries = vec![
            chain.append(rec(AuditKind::FriendAdded, 1)).unwrap(),
            chain.append(rec(AuditKind::FriendAdded, 2)).unwrap(),
            chain.append(rec(AuditKind::FriendAdded, 3)).unwrap(),
        ];
        // Tamper the middle entry's payload.
        entries[1].record.actor_pub = "evil-actor".into();
        let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
        let err = verifier.verify(&entries).unwrap_err();
        match err {
            VerifyError::MacMismatch { cursor, index } => {
                assert_eq!(cursor, 2);
                assert_eq!(index, 1);
            }
            other => panic!("expected MacMismatch, got {other:?}"),
        }
    }

    #[test]
    fn detects_swapped_entries() {
        let key = fixture_key();
        let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
        let e1 = chain.append(rec(AuditKind::FriendAdded, 1)).unwrap();
        let e2 = chain.append(rec(AuditKind::FriendRemoved, 2)).unwrap();
        // Swap order — prev_mac of e1 (genesis) != e2.mac.
        let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
        let err = verifier.verify(&[e2, e1]).unwrap_err();
        assert!(matches!(
            err,
            VerifyError::PrevMacMismatch { .. } | VerifyError::NonMonotonicCursor { .. }
        ));
    }

    #[test]
    fn detects_inserted_entry() {
        let key = fixture_key();
        let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
        let e1 = chain.append(rec(AuditKind::FriendAdded, 1)).unwrap();
        let e3 = chain.append(rec(AuditKind::FriendAdded, 3)).unwrap();
        // Insert a forged entry between e1 and e3. The forger built it
        // against the previous chain tail (e1.mac, cursor 1), so the
        // forged cursor is 2 — which collides with e3.cursor == 2.
        // Verification catches this via NonMonotonicCursor at e3, or
        // PrevMacMismatch (forged.mac ≠ e3.prev_mac) — either is a
        // valid detection.
        let mut forged_chain = AuditChain::open(fixture_key(), e1.mac, 1);
        let forged = forged_chain
            .append(rec(AuditKind::FriendAdded, 99))
            .unwrap();
        let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
        let err = verifier.verify(&[e1, forged, e3]).unwrap_err();
        assert!(matches!(
            err,
            VerifyError::PrevMacMismatch { .. } | VerifyError::NonMonotonicCursor { .. }
        ));
    }

    #[test]
    fn detects_truncated_genesis() {
        let key = fixture_key();
        let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
        let _e1 = chain.append(rec(AuditKind::FriendAdded, 1)).unwrap();
        let e2 = chain.append(rec(AuditKind::FriendAdded, 2)).unwrap();
        // Drop e1 — verifier sees e2.prev_mac != [0; 32].
        let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
        let err = verifier.verify(&[e2]).unwrap_err();
        assert!(matches!(
            err,
            VerifyError::PrevMacMismatch { cursor: 2, .. }
        ));
    }

    #[test]
    fn wrong_key_fails_every_position() {
        let mut chain = AuditChain::open(fixture_key(), [0u8; MAC_LEN], 0);
        let entries = vec![
            chain.append(rec(AuditKind::FriendAdded, 1)).unwrap(),
            chain.append(rec(AuditKind::FriendAdded, 2)).unwrap(),
        ];
        let wrong_key = Zeroizing::new([99u8; KEY_LEN]);
        let verifier = AuditChain::open(wrong_key, [0u8; MAC_LEN], 0);
        let err = verifier.verify(&entries).unwrap_err();
        // Wrong key invalidates the very first MAC.
        assert!(matches!(
            err,
            VerifyError::MacMismatch {
                cursor: 1,
                index: 0
            }
        ));
    }

    #[test]
    fn cursor_overflow_saturates_safely() {
        // Defence-in-depth: append at u64::MAX should not panic.
        let mut chain = AuditChain::open(fixture_key(), [0u8; MAC_LEN], u64::MAX);
        let entry = chain.append(rec(AuditKind::FriendAdded, 1)).unwrap();
        assert_eq!(entry.cursor, u64::MAX, "saturating_add caps at u64::MAX");
    }
}
