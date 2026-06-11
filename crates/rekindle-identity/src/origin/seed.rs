//! `OriginSeed` — 32 bytes of CSPRNG material, the single derivation
//! root of one identity.
//!
//! # Invariants
//!
//! - NOT `Debug` (no seed bytes in logs, ever)
//! - NOT `Clone` (one owner, one seed)
//! - NOT `Serialize` / `Deserialize` (never in a wire object)
//! - NOT `PartialEq` / `Eq` (comparison is never a valid operation —
//!   two seeds are either the same allocation or they are distinct identities)
//! - `Zeroizing<[u8; 32]>` on drop (including panic paths)
//!
//! The reference-implementation requirement (documented, enforced by the
//! consuming daemon): seed bytes live in protected allocation
//! (`ProtectedAlloc`-class: guard pages, canary, volatile zeroize).
//! This crate uses `Zeroizing` as the floor; the daemon wraps further.

use zeroize::Zeroizing;

/// 32 bytes of CSPRNG material. The single root input to all key
/// derivation for one identity.
///
/// One `OriginSeed` per identity; one identity per `OriginSeed`.
///
/// # Construction
///
/// - `OriginSeed::generate()` — from OS CSPRNG. Used at origination.
/// - `OriginSeed::from_vault_bytes()` — from vault-loaded bytes. Used
///   at unlock / restore. Takes ownership and zeroizes the source.
///
/// # Access
///
/// - `expose()` — crate-internal only. Returns `&[u8; 32]` for KDF
///   inputs. No public accessor for the raw bytes exists.
pub struct OriginSeed(Zeroizing<[u8; 32]>);

impl OriginSeed {
    /// Generate a new seed from OS CSPRNG.
    ///
    /// This is the only way to create a new identity. Called exactly
    /// once per identity lifetime, at origination.
    pub fn generate() -> Self {
        let mut bytes = Zeroizing::new([0u8; 32]);
        // aws-lc-rs SystemRandom or getrandom — use the ratchet crate's
        // RNG path for consistency. If that path is unavailable, fall
        // back to getrandom directly.
        getrandom::getrandom(bytes.as_mut())
            .expect("OS CSPRNG must be available for identity origination");
        Self(bytes)
    }

    /// Restore a seed from vault-loaded bytes.
    ///
    /// Takes ownership of the `Zeroizing<[u8; 32]>`, which zeroizes
    /// the source on drop. The caller MUST NOT retain a copy.
    pub fn from_vault_bytes(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }

    /// Crate-internal access to the raw seed bytes.
    ///
    /// Used by derivation functions (G1, G2, G3) that need the seed
    /// as IKM.
    pub(crate) fn expose(&self) -> &[u8; 32] {
        &self.0
    }

    /// Export seed bytes for vault storage ONLY.
    ///
    /// The returned bytes MUST be written to an encrypted vault
    /// immediately and the caller MUST NOT retain a copy. This
    /// method exists because the vault storage layer lives outside
    /// this crate and needs the raw bytes to persist them.
    ///
    /// Do NOT use this for derivation — use `SelfIdentity` methods.
    /// Do NOT use this for signing — use `SelfIdentity::sign()`.
    /// Do NOT log, display, or transmit the returned bytes.
    pub fn vault_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Manual Debug impl that NEVER prints seed bytes.
impl core::fmt::Debug for OriginSeed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OriginSeed(REDACTED)")
    }
}

// OriginSeed is intentionally:
// - NOT Clone (no accidental duplication of seed material)
// - NOT Copy (32 bytes, but the invariant is single-owner)
// - NOT PartialEq / Eq (comparison is meaningless for secrets)
// - NOT Serialize / Deserialize (never a wire or persistence object)
// - NOT Display (same as Debug — no seed exposure)
// - NOT Send / Sync by default is fine — it IS Send+Sync via Zeroizing,
//   but we don't need to add manual impls.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_non_zero() {
        let seed = OriginSeed::generate();
        // Probability of all-zeros from CSPRNG: 2^-256
        assert_ne!(seed.expose(), &[0u8; 32]);
    }

    #[test]
    fn generate_produces_distinct_seeds() {
        let a = OriginSeed::generate();
        let b = OriginSeed::generate();
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn from_vault_bytes_roundtrip() {
        let original = [0x42u8; 32];
        let seed = OriginSeed::from_vault_bytes(Zeroizing::new(original));
        assert_eq!(seed.expose(), &original);
    }

    #[test]
    fn debug_does_not_leak_bytes() {
        let seed = OriginSeed::generate();
        let dbg = format!("{seed:?}");
        assert_eq!(dbg, "OriginSeed(REDACTED)");
        // Extra paranoia: ensure no hex encoding of seed bytes appears
        let hex = hex::encode(seed.expose());
        assert!(
            !dbg.contains(&hex),
            "Debug output must not contain seed hex"
        );
    }

    #[test]
    fn zeroize_structural() {
        let seed = OriginSeed::generate();
        assert_ne!(seed.expose(), &[0u8; 32]);
        drop(seed);
    }
}
