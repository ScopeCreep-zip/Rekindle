//! Session anchor — the canonical 32-byte session identifier between two peers.
//!
//! `session_anchor()` is the ONLY function in the workspace that produces
//! session identifiers. Both sides of a handshake call it with the same
//! two roots and get the same output, regardless of who initiated.

use crate::error::IdentityError;
use crate::origin::originate::IdentityRoot;
use crate::origin::tags::derivation_tags;

/// Canonical session identifier between two peers.
///
/// Produced by `session_anchor()` only. Used as the key for ratchet
/// session storage, DM log association, and session-cache lookup.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionAnchor([u8; 32]);

impl SessionAnchor {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl core::fmt::Debug for SessionAnchor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let h = self.to_hex();
        write!(f, "Anchor({}…{})", &h[..8], &h[56..])
    }
}

/// G6: Derive the canonical session anchor between two identity roots.
///
/// Lexicographically orders the two raw 32-byte roots, concatenates
/// into a 64-byte buffer (stack-allocated, zero heap), and applies
/// a domain-separated KDF.
///
/// Properties (tested):
/// - `session_anchor(a, b) == session_anchor(b, a)` for all pairs
/// - Distinct pairs collide only at BLAKE3 collision odds
/// - `a == b` is rejected (`IdentityError::SelfSession`)
///
/// Inputs are raw 32-byte public key material only — never hex strings,
/// never seeds, never mixed encodings. The function signature enforces
/// this: `&IdentityRoot` cannot be constructed from a seed or unvalidated
/// string.
pub fn session_anchor(
    a: &IdentityRoot,
    b: &IdentityRoot,
) -> Result<SessionAnchor, IdentityError> {
    if a == b {
        return Err(IdentityError::SelfSession);
    }

    // Lexicographic ordering over raw bytes ensures both sides
    // produce the same anchor regardless of who is "a" and "b".
    let (lo, hi) = if a.as_bytes() < b.as_bytes() {
        (a.as_bytes(), b.as_bytes())
    } else {
        (b.as_bytes(), a.as_bytes())
    };

    // Stack-allocated 64-byte concatenation. Zero heap allocation.
    let mut ikm = [0u8; 64];
    ikm[..32].copy_from_slice(lo);
    ikm[32..].copy_from_slice(hi);

    let derived = blake3::derive_key(derivation_tags::SESSION_ANCHOR, &ikm);
    Ok(SessionAnchor(derived))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn root_from_seed(byte: u8) -> IdentityRoot {
        originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([byte; 32]))
        ).unwrap().root
    }

    #[test]
    fn symmetric() {
        let a = root_from_seed(0x01);
        let b = root_from_seed(0x02);
        assert_eq!(
            session_anchor(&a, &b).unwrap(),
            session_anchor(&b, &a).unwrap(),
            "session_anchor must be symmetric"
        );
    }

    #[test]
    fn distinct_pairs_distinct_anchors() {
        let a = root_from_seed(0x01);
        let b = root_from_seed(0x02);
        let c = root_from_seed(0x03);

        let ab = session_anchor(&a, &b).unwrap();
        let ac = session_anchor(&a, &c).unwrap();
        let bc = session_anchor(&b, &c).unwrap();

        assert_ne!(ab, ac);
        assert_ne!(ab, bc);
        assert_ne!(ac, bc);
    }

    #[test]
    fn self_session_rejected() {
        let a = root_from_seed(0x01);
        let err = session_anchor(&a, &a).unwrap_err();
        assert!(matches!(err, IdentityError::SelfSession));
    }

    #[test]
    fn deterministic() {
        let a = root_from_seed(0x01);
        let b = root_from_seed(0x02);
        let anchor1 = session_anchor(&a, &b).unwrap();
        let anchor2 = session_anchor(&a, &b).unwrap();
        assert_eq!(anchor1, anchor2);
    }

    #[test]
    fn anchor_is_32_bytes() {
        let a = root_from_seed(0x01);
        let b = root_from_seed(0x02);
        let anchor = session_anchor(&a, &b).unwrap();
        assert_eq!(anchor.as_bytes().len(), 32);
    }

    #[test]
    fn hex_roundtrip() {
        let a = root_from_seed(0x01);
        let b = root_from_seed(0x02);
        let anchor = session_anchor(&a, &b).unwrap();
        let hex = anchor.to_hex();
        assert_eq!(hex.len(), 64);
    }

    /// Symmetry over many random-ish pairs.
    #[test]
    fn symmetry_1000_pairs() {
        for i in 0..250u8 {
            for j in (i + 1)..=250u8.min(i + 4) {
                let a = root_from_seed(i);
                let b = root_from_seed(j);
                assert_eq!(
                    session_anchor(&a, &b).unwrap(),
                    session_anchor(&b, &a).unwrap(),
                    "symmetry failed for pair ({i}, {j})"
                );
            }
        }
    }
}
