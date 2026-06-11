//! Derivation tag constants — frozen `&'static str` context strings.
//!
//! Every `blake3::derive_key(CONTEXT, IKM)` call in the identity crate
//! uses a constant from this module as CONTEXT. User-influenced data is
//! NEVER placed in CONTEXT — it goes in IKM exclusively.
//!
//! These strings are the identity protocol's KDF grammar. Changing any
//! string is a wire-breaking change that invalidates every derived key
//! in every vault and every published pseudonym. They are frozen.

/// Derivation tag constants for all identity KDF operations.
///
/// Each constant is the CONTEXT argument to `blake3::derive_key`.
/// The IKM (input keying material) for each is documented in
/// RID-SPEC-001 §3.1 derivation table.
pub mod derivation_tags {
    /// G2/G5: X25519 DH scalar from a 32-byte seed (OriginSeed or pseudonym seed).
    ///
    /// INHERITED VERBATIM from the live PQXDH flow:
    /// `rekindle-chat/src/crypto/mod.rs:52` and `accept.rs:93`.
    /// Changing this breaks every published DhKey and every active
    /// PQXDH handshake. FROZEN.
    pub const DH_FROM_SEED: &str = "rekindle identity x25519 v1";

    /// G3: Pseudonym seed derivation.
    ///
    /// v2 grammar: FIXED context string, governance key in IKM.
    /// Supersedes v1 which interpolated the governance key into the
    /// context string (violating BLAKE3 derive_key convention) and
    /// did not canonicalize the substrate prefix. Pre-release, no
    /// migration needed.
    pub const PSEUDONYM_SEED: &str = "rekindle identity pseudonym v2";

    /// G6: Session anchor between two identity roots.
    pub const SESSION_ANCHOR: &str = "rekindle identity session-anchor v1";

    /// Device cross-sign message domain.
    pub const DEVICE_SIGN: &str = "rekindle identity device-sign v1";

    /// Rotation proof message domain.
    pub const ROTATION_SIGN: &str = "rekindle identity rotation v1";

    /// Revocation certificate message domain.
    pub const REVOCATION_SIGN: &str = "rekindle identity revocation v1";

    /// Death notice message domain.
    pub const DEATH_SIGN: &str = "rekindle identity death v1";

    /// LinkageProof: root-signs-pseudonym direction.
    pub const LINKAGE_ROOT_SIGN: &str = "rekindle identity linkage-root v1";

    /// LinkageProof: pseudonym-signs-root direction.
    pub const LINKAGE_PSEUDONYM_SIGN: &str = "rekindle identity linkage-pseudonym v1";

    /// DelegationGrant message domain.
    pub const GRANT_SIGN: &str = "rekindle identity grant v1";

    /// PrekeyBundleBinding message domain.
    pub const PREKEY_BINDING_SIGN: &str = "rekindle identity prekey-binding v1";

    /// LocatorRecord message domain.
    pub const LOCATOR_SIGN: &str = "rekindle identity locator v1";

    /// DisplayName signed claim domain.
    pub const DISPLAY_NAME_SIGN: &str = "rekindle identity display-name v1";

    /// Community member slot seed.
    pub const SLOT_SEED: &str = "rekindle identity slot-seed v1";
}

#[cfg(test)]
mod tests {
    use super::derivation_tags::*;

    /// Every tag must be unique. A collision means two derivations
    /// produce the same output from the same IKM — a catastrophic
    /// domain-separation failure.
    #[test]
    fn all_tags_unique() {
        let tags = [
            DH_FROM_SEED,
            PSEUDONYM_SEED,
            SESSION_ANCHOR,
            DEVICE_SIGN,
            ROTATION_SIGN,
            REVOCATION_SIGN,
            DEATH_SIGN,
            LINKAGE_ROOT_SIGN,
            LINKAGE_PSEUDONYM_SIGN,
            GRANT_SIGN,
            PREKEY_BINDING_SIGN,
            LOCATOR_SIGN,
            DISPLAY_NAME_SIGN,
            SLOT_SEED,
        ];
        let mut seen = std::collections::HashSet::new();
        for tag in &tags {
            assert!(
                seen.insert(*tag),
                "DUPLICATE derivation tag: {tag}"
            );
        }
    }

    /// Every tag must start with "rekindle identity " — namespace discipline.
    #[test]
    fn all_tags_namespaced() {
        let tags = [
            DH_FROM_SEED,
            PSEUDONYM_SEED,
            SESSION_ANCHOR,
            DEVICE_SIGN,
            ROTATION_SIGN,
            REVOCATION_SIGN,
            DEATH_SIGN,
            LINKAGE_ROOT_SIGN,
            LINKAGE_PSEUDONYM_SIGN,
            GRANT_SIGN,
            PREKEY_BINDING_SIGN,
            LOCATOR_SIGN,
            DISPLAY_NAME_SIGN,
            SLOT_SEED,
        ];
        for tag in &tags {
            assert!(
                tag.starts_with("rekindle identity "),
                "tag does not start with 'rekindle identity ': {tag}"
            );
        }
    }

    /// The DH_FROM_SEED tag must match the live PQXDH convention exactly.
    /// This is a compatibility gate — if it differs, every handshake breaks.
    #[test]
    fn dh_tag_matches_live_convention() {
        assert_eq!(DH_FROM_SEED, "rekindle identity x25519 v1");
    }

    /// Every tag must be non-empty and contain only printable ASCII.
    #[test]
    fn all_tags_printable_ascii() {
        let tags = [
            DH_FROM_SEED, PSEUDONYM_SEED, SESSION_ANCHOR,
            DEVICE_SIGN, ROTATION_SIGN, REVOCATION_SIGN, DEATH_SIGN,
            LINKAGE_ROOT_SIGN, LINKAGE_PSEUDONYM_SIGN,
            GRANT_SIGN, PREKEY_BINDING_SIGN, LOCATOR_SIGN,
            DISPLAY_NAME_SIGN, SLOT_SEED,
        ];
        for tag in &tags {
            assert!(!tag.is_empty(), "empty tag");
            assert!(
                tag.bytes().all(|b| b >= 0x20 && b < 0x7F),
                "non-printable ASCII in tag: {tag}"
            );
        }
    }
}
