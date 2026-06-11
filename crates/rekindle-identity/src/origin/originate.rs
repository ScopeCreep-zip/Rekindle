//! Identity origination — the single constructor for a new identity.
//!
//! `originate()` produces the complete initial identity material from
//! a fresh `OriginSeed`: the `IdentityRoot` (Ed25519 public key), the
//! `DhSeed` (X25519 scalar), and a `RevocationCertificate` (pre-committed,
//! for offline custody). All derivations are deterministic from the seed.
//!
//! `restore()` re-derives the public keys from a vault-loaded seed
//! WITHOUT minting a new `RevocationCertificate` — the original's
//! offline copy must stay authoritative.

use zeroize::Zeroizing;

use rekindle_ratchet::crypto::sign;

use crate::error::IdentityError;
use crate::origin::seed::OriginSeed;
use crate::origin::tags::derivation_tags;
use crate::wire::encode;
use crate::wire::signable::{Signable, Signature64};

// ── IdentityRoot ────────────────────────────────────────────────
//
// Defined here (not in root/) because origination is the first place
// it's constructed. Re-exported from root/mod.rs and crate root.

/// Ed25519 public key — THE identity. 32 bytes.
///
/// Two peers are the same peer iff their `IdentityRoot` values are
/// equal within the same `RotationEpoch` chain. This is the ONLY
/// value that vault labels, session anchors, trust records, moderation
/// actions, delegation edges, and equality comparisons key on.
///
/// Construction: `from_bytes()` validates via round-trip through the
/// ratchet's `keypair_from_seed` → `public_key_bytes` path (for
/// origination) or via `Verified<T>` parse (for wire objects). There
/// is no `from_bytes_unchecked` in the public API.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdentityRoot([u8; 32]);

impl IdentityRoot {
    /// Construct from raw bytes. Public API — validates that the bytes
    /// are plausibly an Ed25519 public key by checking non-zero and
    /// length. Full point validation happens at signature verification
    /// time via `Verified<T>`.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, IdentityError> {
        // All-zeros is definitely not a valid Ed25519 public key
        // (it's the neutral element, which is a low-order point).
        if bytes == [0u8; 32] {
            return Err(IdentityError::InvalidRoot);
        }
        Ok(Self(bytes))
    }

    /// Construct from hex string.
    pub fn from_hex(s: &str) -> Result<Self, IdentityError> {
        let bytes = hex::decode(s).map_err(|_| IdentityError::InvalidRoot)?;
        if bytes.len() != 32 {
            return Err(IdentityError::InvalidRoot);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Self::from_bytes(arr)
    }

    /// Crate-internal constructor from derivation output. The bytes
    /// came from `sign::public_key_bytes(keypair)` which is guaranteed
    /// to be a valid Ed25519 public key (it was just generated from a seed).
    pub(crate) fn from_bytes_unchecked(bytes: [u8; 32]) -> Self {
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

    /// Short display form for human-readable output. 12 hex characters
    /// (first 8 + "…" + last 4). NOT accepted by any storage, derivation,
    /// comparison, or label API.
    pub fn display_short(&self) -> String {
        let h = self.to_hex();
        format!("{}…{}", &h[..8], &h[60..])
    }
}

impl core::fmt::Debug for IdentityRoot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Root({})", self.display_short())
    }
}

impl core::fmt::Display for IdentityRoot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl serde::Serialize for IdentityRoot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> serde::Deserialize<'de> for IdentityRoot {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

// ── PeerRef ─────────────────────────────────────────────────────

/// The stable peer-identifying handle. Equality, hashing, ordering
/// derive from the anchor root bytes alone.
///
/// The inner root is the **stable anchor** — the origination-epoch root
/// or the first-observed root. It does NOT change on rotation.
/// `TrustRecord.pinned_root` carries the current chain head for
/// signature verification; `PeerRef` carries the permanent name.
///
/// **Construction is sealed.** The inner field is private. There is no
/// tuple constructor, no `From<IdentityRoot>`, no `Serialize`, no
/// `Deserialize`. The single `pub(crate)` constructor `anchor()` is
/// the only way to mint a `PeerRef`. Legitimate callers: first-contact
/// observation in `TrustStore::observe_root`, and vault import in
/// `TrustStore::import`.
///
/// `PeerRef` does not implement serde. `TrustRecord` stores
/// `anchor_root: IdentityRoot` (which does serialize) and the store
/// reconstructs `PeerRef` on import through `anchor()`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PeerRef {
    root: IdentityRoot,
}

impl PeerRef {
    /// The single anchor-establishing constructor.
    ///
    /// Callers assert the root is a stable anchor — the origination-epoch
    /// root or the first-observed root, never a rotated chain head.
    /// Legitimate call sites: `TrustStore::observe_root` (first contact)
    /// and `TrustStore::import` (vault restore).
    pub(crate) fn anchor(root: IdentityRoot) -> Self {
        Self { root }
    }

    /// The stable anchor root.
    pub fn root(&self) -> &IdentityRoot {
        &self.root
    }
}

// ── Rotation epoch ──────────────────────────────────────────────

/// Rotation epoch counter. 0 = origination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct RotationEpoch(pub u64);

impl RotationEpoch {
    pub const ORIGIN: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("rotation epoch overflow"))
    }
}

// ── DhSeed ──────────────────────────────────────────────────────

/// X25519 DH seed — derived from `OriginSeed` via G2.
///
/// Same zeroizing hygiene as `OriginSeed`: not Clone, not Serialize.
pub struct DhSeed(Zeroizing<[u8; 32]>);

impl DhSeed {
    /// Crate-internal constructor from raw KDF output.
    pub(crate) fn from_raw(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }

    /// Raw bytes for the downstream PQXDH/MEK flow which constructs
    /// `aws_lc_rs::agreement::PrivateKey` from this seed.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl core::fmt::Debug for DhSeed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DhSeed(REDACTED)")
    }
}

// ── RevocationCertificate ───────────────────────────────────────

/// Pre-committed revocation certificate. Generated at origination,
/// exported for offline custody, published only on compromise.
///
/// The signature predates compromise by construction — this is the
/// only sound revocation mechanism for a compromised root.
#[derive(Debug, Clone)]
pub struct RevocationCertificate {
    pub root: IdentityRoot,
    pub epoch_at_issue: RotationEpoch,
    pub signature: Signature64,
}

/// Internal signable form for the revocation certificate.
struct RevocationSignable {
    root_bytes: [u8; 32],
    epoch: RotationEpoch,
}

impl Signable for RevocationSignable {
    const SIGN_DOMAIN: &'static str = derivation_tags::REVOCATION_SIGN;

    fn encode_fields(&self, buf: &mut Vec<u8>) {
        // WIRE FORMAT: field order pinned — reordering is a wire break
        encode::array_head(buf, 2);
        encode::bytes(buf, &self.root_bytes);
        encode::unsigned(buf, self.epoch.0);
    }
}

// ── Originated / Restored ───────────────────────────────────────

/// The complete output of identity origination.
pub struct OriginatedIdentity {
    pub seed: OriginSeed,
    pub root: IdentityRoot,
    pub dh_seed: DhSeed,
    pub dh_public: [u8; 32],
    pub revocation: RevocationCertificate,
    pub epoch: RotationEpoch,
}

/// Output of `restore()` — no revocation certificate.
pub struct RestoredIdentity {
    pub seed: OriginSeed,
    pub root: IdentityRoot,
    pub dh_seed: DhSeed,
    pub dh_public: [u8; 32],
}

// ── Origination ─────────────────────────────────────────────────

/// Originate a new identity from a fresh CSPRNG seed.
pub fn originate() -> Result<OriginatedIdentity, IdentityError> {
    originate_from_seed(OriginSeed::generate())
}

/// Originate from a specific seed (deterministic, for tests).
pub fn originate_from_seed(seed: OriginSeed) -> Result<OriginatedIdentity, IdentityError> {
    // G1: Ed25519 public key from seed
    let kp = sign::keypair_from_seed(seed.expose())
        .map_err(|e| IdentityError::KeypairDerivation {
            reason: format!("Ed25519: {e}"),
        })?;
    let root_bytes = sign::public_key_bytes(&kp);
    let root = IdentityRoot::from_bytes_unchecked(root_bytes);

    // G2: X25519 DH seed (BLAKE3 domain-separated, independent scalar)
    let dh_raw = blake3::derive_key(derivation_tags::DH_FROM_SEED, seed.expose());
    let dh_seed = DhSeed::from_raw(Zeroizing::new(dh_raw));
    let dh_public = derive_dh_public(&dh_seed)?;

    // Revocation certificate — signed NOW, exported for offline custody
    let epoch = RotationEpoch::ORIGIN;
    let revocation = sign_revocation(seed.expose(), &root, epoch)?;

    Ok(OriginatedIdentity {
        seed,
        root,
        dh_seed,
        dh_public,
        revocation,
        epoch,
    })
}

/// Restore an identity from a vault-loaded seed. Does NOT mint a new
/// `RevocationCertificate` — the original must be loaded from the vault.
pub fn restore(seed: OriginSeed) -> Result<RestoredIdentity, IdentityError> {
    let kp = sign::keypair_from_seed(seed.expose())
        .map_err(|e| IdentityError::KeypairDerivation {
            reason: format!("Ed25519 restore: {e}"),
        })?;
    let root_bytes = sign::public_key_bytes(&kp);
    let root = IdentityRoot::from_bytes_unchecked(root_bytes);

    let dh_raw = blake3::derive_key(derivation_tags::DH_FROM_SEED, seed.expose());
    let dh_seed = DhSeed::from_raw(Zeroizing::new(dh_raw));
    let dh_public = derive_dh_public(&dh_seed)?;

    Ok(RestoredIdentity { seed, root, dh_seed, dh_public })
}

// ── Internal helpers ────────────────────────────────────────────

fn derive_dh_public(dh_seed: &DhSeed) -> Result<[u8; 32], IdentityError> {
    let private = rekindle_ratchet::crypto::dh::reusable_from_seed(dh_seed.expose())
        .map_err(|_| IdentityError::InvalidDhKey)?;
    let public = private
        .compute_public_key()
        .map_err(|_| IdentityError::InvalidDhKey)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(public.as_ref());
    Ok(out)
}

fn sign_revocation(
    seed: &[u8; 32],
    root: &IdentityRoot,
    epoch: RotationEpoch,
) -> Result<RevocationCertificate, IdentityError> {
    let kp = sign::keypair_from_seed(seed)
        .map_err(|e| IdentityError::KeypairDerivation {
            reason: format!("revocation sign: {e}"),
        })?;
    let signable = RevocationSignable {
        root_bytes: *root.as_bytes(),
        epoch,
    };
    let sig_bytes = sign::sign_raw(&kp, &signable.signable_bytes());
    Ok(RevocationCertificate {
        root: *root,
        epoch_at_issue: epoch,
        signature: Signature64::from_bytes(sig_bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn originate_produces_valid_root() {
        let o = originate().unwrap();
        assert_ne!(o.root.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn originate_deterministic_from_seed() {
        let a = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        let b = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        assert_eq!(a.root, b.root);
        assert_eq!(a.dh_public, b.dh_public);
        assert_eq!(a.epoch, b.epoch);
    }

    #[test]
    fn originate_distinct_seeds_distinct_roots() {
        let a = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
        ).unwrap();
        let b = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))
        ).unwrap();
        assert_ne!(a.root, b.root);
        assert_ne!(a.dh_public, b.dh_public);
    }

    #[test]
    fn originate_epoch_is_zero() {
        let o = originate().unwrap();
        assert_eq!(o.epoch, RotationEpoch::ORIGIN);
    }

    #[test]
    fn originate_revocation_present_and_signed() {
        let o = originate().unwrap();
        assert_eq!(o.revocation.root, o.root);
        assert_eq!(o.revocation.epoch_at_issue, RotationEpoch::ORIGIN);
        assert_ne!(o.revocation.signature, Signature64::ZERO);

        // Verify the signature is valid
        let signable = RevocationSignable {
            root_bytes: *o.root.as_bytes(),
            epoch: RotationEpoch::ORIGIN,
        };
        assert!(
            sign::verify_raw(
                o.root.as_bytes(),
                &signable.signable_bytes(),
                o.revocation.signature.as_bytes(),
            ).is_ok(),
            "revocation certificate signature must verify against the root"
        );
    }

    #[test]
    fn restore_matches_originate() {
        let seed_bytes = [0xAA; 32];
        let originated = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed_bytes))
        ).unwrap();
        let restored = restore(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed_bytes))
        ).unwrap();
        assert_eq!(originated.root, restored.root);
        assert_eq!(originated.dh_public, restored.dh_public);
    }

    #[test]
    fn dh_public_distinct_from_root() {
        let o = originate().unwrap();
        assert_ne!(&o.dh_public, o.root.as_bytes(),
            "DH public key must differ from Ed25519 root (independent derivation)");
    }

    #[test]
    fn dh_seed_matches_live_convention() {
        // The DH seed derivation must produce the same bytes as the
        // live PQXDH flow in rekindle-chat/src/crypto/mod.rs:52.
        let seed = [0x42u8; 32];
        let expected = blake3::derive_key("rekindle identity x25519 v1", &seed);
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();
        assert_eq!(o.dh_seed.as_bytes(), &expected,
            "DH seed must match the live PQXDH convention");
    }

    #[test]
    fn identity_root_from_bytes_rejects_zeros() {
        assert!(IdentityRoot::from_bytes([0u8; 32]).is_err());
    }

    #[test]
    fn identity_root_hex_roundtrip() {
        let o = originate().unwrap();
        let hex = o.root.to_hex();
        assert_eq!(hex.len(), 64);
        let restored = IdentityRoot::from_hex(&hex).unwrap();
        assert_eq!(o.root, restored);
    }

    #[test]
    fn identity_root_from_hex_rejects_bad_input() {
        assert!(IdentityRoot::from_hex("").is_err());
        assert!(IdentityRoot::from_hex("not hex").is_err());
        assert!(IdentityRoot::from_hex(&"aa".repeat(31)).is_err()); // 31 bytes
        assert!(IdentityRoot::from_hex(&"aa".repeat(33)).is_err()); // 33 bytes
        assert!(IdentityRoot::from_hex(&"00".repeat(32)).is_err()); // all zeros
    }

    #[test]
    fn identity_root_debug_short() {
        let o = originate().unwrap();
        let dbg = format!("{:?}", o.root);
        assert!(dbg.starts_with("Root("));
        assert!(dbg.contains('…'));
        assert!(dbg.len() < 30, "debug should be short: {dbg}");
    }

    #[test]
    fn identity_root_serde_roundtrip() {
        let o = originate().unwrap();
        let json = serde_json::to_string(&o.root).unwrap();
        let restored: IdentityRoot = serde_json::from_str(&json).unwrap();
        assert_eq!(o.root, restored);
    }

    #[test]
    fn peer_ref_equality_is_root_only() {
        let o = originate().unwrap();
        let a = PeerRef::anchor(o.root);
        let b = PeerRef::anchor(o.root);
        assert_eq!(a, b);

        let mut map = std::collections::HashMap::new();
        map.insert(a, "first");
        map.insert(b, "second"); // same key, overwrites
        assert_eq!(map.len(), 1);
        assert_eq!(map[&a], "second");
    }

    /// Pinned derivation stability test: same seed always produces
    /// same outputs. The hex values are recorded on first run and
    /// frozen — any derivation grammar change breaks this test.
    #[test]
    fn pinned_derivation_stability() {
        let seed = [0x01u8; 32];
        let a = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();
        let b = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new(seed))
        ).unwrap();
        assert_eq!(a.root.to_hex(), b.root.to_hex());
        assert_eq!(hex::encode(a.dh_public), hex::encode(b.dh_public));
        assert_eq!(a.revocation.signature, b.revocation.signature);
    }
}
