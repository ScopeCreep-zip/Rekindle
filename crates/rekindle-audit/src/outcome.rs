//! Restoring and verifying a chain, as functions over values with typed
//! outcomes (plan D1). The I/O around them — the database rows and the
//! vault's key and tail anchor — is the host's (`rekindle_node::host::audit`).

use zeroize::Zeroizing;

use crate::{AuditChain, AuditEntry, Tail, TailCheck, VerifyError};

/// What restoring a chain found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restored {
    /// The stored tail is the one the vault anchored (or no anchor exists
    /// yet, or the anchor is merely behind after a lost vault write).
    Intact,
    /// Rows were removed or the last one rewritten after the anchored
    /// entry: the chain continues from `anchor_cursor`, the last entry
    /// known good.
    Truncated { anchor_cursor: u64 },
}

/// Open the chain that continues from the stored tail, or from the vault's
/// anchor when the stored tail cannot be trusted.
#[must_use]
pub fn restore(
    key: Zeroizing<[u8; 32]>,
    anchor: Option<Tail>,
    stored: Tail,
) -> (AuditChain, Restored) {
    let check = TailCheck::of(anchor, stored);
    let (cursor, last_mac) = check.resume_from(stored);
    let restored = match check {
        TailCheck::Tampered { anchor } => Restored::Truncated {
            anchor_cursor: anchor.0,
        },
        TailCheck::Unanchored | TailCheck::Clean | TailCheck::CatchUp => Restored::Intact,
    };
    (AuditChain::open(key, last_mac, cursor), restored)
}

/// What verifying a chain found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Verified {
    Intact {
        length: u64,
    },
    Broken {
        length: u64,
        /// The first entry that fails, when one does (a serialization
        /// failure names none).
        at: Option<u64>,
        detail: String,
    },
}

/// Verify `entries` against `chain` end to end.
#[must_use]
pub fn verify(chain: &AuditChain, entries: &[AuditEntry]) -> Verified {
    let length = entries.len() as u64;
    match chain.verify(entries) {
        Ok(()) => Verified::Intact { length },
        Err(e) => {
            let at = match &e {
                VerifyError::PrevMacMismatch { cursor, .. }
                | VerifyError::MacMismatch { cursor, .. }
                | VerifyError::NonMonotonicCursor { cursor, .. } => Some(*cursor),
                VerifyError::Serialize(_) => None,
            };
            Verified::Broken {
                length,
                at,
                detail: e.to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuditKind, AuditRecord};

    fn record() -> AuditRecord {
        AuditRecord {
            at_ms: 1,
            actor_pub: "me".into(),
            kind: AuditKind::VaultUnlocked,
            payload: serde_json::json!({}),
        }
    }

    #[test]
    fn a_tampered_entry_is_broken_and_a_removed_tail_is_truncated() {
        let (mut chain, restored) = restore(Zeroizing::new([7; 32]), None, (0, [0; 32]));
        assert_eq!(restored, Restored::Intact);
        let first = chain.append(record()).unwrap();
        let second = chain.append(record()).unwrap();
        assert_eq!(
            verify(&chain, &[first.clone(), second.clone()]),
            Verified::Intact { length: 2 }
        );
        let mut forged = second.clone();
        forged.record.payload = serde_json::json!({ "forged": true });
        assert!(matches!(
            verify(&chain, &[first.clone(), forged]),
            Verified::Broken { at: Some(_), .. }
        ));
        // The vault anchored the second entry; the store ends at the first.
        let (_, restored) = restore(
            Zeroizing::new([7; 32]),
            Some((second.cursor, second.mac)),
            (first.cursor, first.mac),
        );
        assert_eq!(
            restored,
            Restored::Truncated {
                anchor_cursor: second.cursor
            }
        );
    }
}
