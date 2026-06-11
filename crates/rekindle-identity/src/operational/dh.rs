//! X25519 DH key types and derivation.
//!
//! `DhKey` is the public X25519 key for Diffie-Hellman operations
//! (PQXDH identity DH, MEK wrapping). `DhSeed` is the private scalar
//! derived from `OriginSeed` via G2 (or pseudonym seed via G5).
//!
//! The derivation convention is BLAKE3 domain-separated independent
//! scalar — the birational Edwards→Montgomery map is prohibited (R-06).
//! aws-lc-rs clamps internally; the canonical private key is the raw
//! KDF output.

use rekindle_ratchet::crypto::dh as ratchet_dh;

use crate::error::IdentityError;
use crate::origin::originate::DhSeed;

/// X25519 public key — 32 bytes.
///
/// Used for PQXDH identity DH and MEK ECDH wrapping. Distinct newtype
/// from `IdentityRoot` (Ed25519) — the compiler prevents passing one
/// where the other is expected.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DhKey([u8; 32]);

impl DhKey {
    /// Construct from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Raw 32-byte public key.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Hex-encoded public key (64 lowercase hex characters).
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Construct from hex string.
    pub fn from_hex(s: &str) -> Result<Self, IdentityError> {
        let bytes = hex::decode(s).map_err(|_| IdentityError::InvalidDhKey)?;
        if bytes.len() != 32 {
            return Err(IdentityError::InvalidDhKey);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl core::fmt::Debug for DhKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let h = self.to_hex();
        write!(f, "DhKey({}…{})", &h[..8], &h[56..])
    }
}

impl serde::Serialize for DhKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> serde::Deserialize<'de> for DhKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

/// Derive the X25519 public key from a `DhSeed`.
///
/// Constructs an `aws_lc_rs::agreement::PrivateKey` from the seed
/// via `rekindle_ratchet::crypto::dh::reusable_from_seed`, then
/// extracts the public key. The private key is dropped immediately.
pub fn dh_public_from_seed(seed: &DhSeed) -> Result<DhKey, IdentityError> {
    let private = ratchet_dh::reusable_from_seed(seed.expose())
        .map_err(|_| IdentityError::InvalidDhKey)?;
    let public = private
        .compute_public_key()
        .map_err(|_| IdentityError::InvalidDhKey)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(public.as_ref());
    Ok(DhKey(out))
}

/// Derive the X25519 seed bytes from any 32-byte seed via G2.
///
/// Used by community operations that derive X25519 keys from pseudonym
/// seeds for MEK wrapping ECDH. The returned bytes are the raw X25519
/// private scalar suitable for `reusable_from_seed()`.
pub fn x25519_seed_from(seed: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key(crate::origin::tags::derivation_tags::DH_FROM_SEED, seed)
}

/// Perform X25519 DH agreement between our seed and a peer's DhKey.
///
/// Returns the 32-byte shared secret wrapped in `Zeroizing`.
/// The private key is reconstructed from seed, used once, and dropped.
pub fn dh_agree(
    our_seed: &DhSeed,
    their_public: &DhKey,
) -> Result<zeroize::Zeroizing<[u8; 32]>, IdentityError> {
    ratchet_dh::ratchet_agree(
        &ratchet_dh::reusable_from_seed(our_seed.expose())
            .map_err(|_| IdentityError::InvalidDhKey)?,
        their_public.as_bytes(),
    ).map_err(|_| IdentityError::InvalidDhKey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use crate::origin::tags::derivation_tags;
    use zeroize::Zeroizing;

    #[test]
    fn dh_public_from_originate() {
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        let dh_key = dh_public_from_seed(&o.dh_seed).unwrap();
        assert_eq!(dh_key.as_bytes(), &o.dh_public);
    }

    #[test]
    fn dh_key_distinct_from_root() {
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x42; 32]))
        ).unwrap();
        let dh_key = dh_public_from_seed(&o.dh_seed).unwrap();
        assert_ne!(dh_key.as_bytes(), o.root.as_bytes());
    }

    #[test]
    fn dh_agreement_symmetric() {
        let o_a = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        let o_b = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))
        ).unwrap();

        let dh_a = dh_public_from_seed(&o_a.dh_seed).unwrap();
        let dh_b = dh_public_from_seed(&o_b.dh_seed).unwrap();

        let shared_ab = dh_agree(&o_a.dh_seed, &dh_b).unwrap();
        let shared_ba = dh_agree(&o_b.dh_seed, &dh_a).unwrap();

        assert_eq!(*shared_ab, *shared_ba, "DH agreement must be symmetric");
    }

    #[test]
    fn dh_seed_matches_live_convention() {
        let seed = [0x99u8; 32];
        let expected = blake3::derive_key(derivation_tags::DH_FROM_SEED, &seed);
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();
        assert_eq!(o.dh_seed.expose(), &expected);
    }

    #[test]
    fn dh_key_hex_roundtrip() {
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        let dh_key = dh_public_from_seed(&o.dh_seed).unwrap();
        let hex = dh_key.to_hex();
        let restored = DhKey::from_hex(&hex).unwrap();
        assert_eq!(dh_key, restored);
    }

    #[test]
    fn dh_key_from_hex_rejects_bad_input() {
        assert!(DhKey::from_hex("").is_err());
        assert!(DhKey::from_hex("not_hex").is_err());
        assert!(DhKey::from_hex(&"aa".repeat(31)).is_err());
        assert!(DhKey::from_hex(&"aa".repeat(33)).is_err());
    }

    #[test]
    fn dh_key_serde_roundtrip() {
        let dh_key = DhKey::from_bytes([0xAB; 32]);
        let json = serde_json::to_string(&dh_key).unwrap();
        let restored: DhKey = serde_json::from_str(&json).unwrap();
        assert_eq!(dh_key, restored);
    }

    #[test]
    fn dh_key_debug_short() {
        let dh_key = DhKey::from_bytes([0xAB; 32]);
        let dbg = format!("{dh_key:?}");
        assert!(dbg.starts_with("DhKey("));
        assert!(dbg.contains('…'));
    }
}
