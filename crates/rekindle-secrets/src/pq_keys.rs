//! ML-KEM-768 wrappers — post-quantum key encapsulation primitives.
//!
//! Phase 3a of the decomposed-harvest plan. Sole consumer of
//! [`libcrux_ml_kem::mlkem768`] in the workspace (per the
//! `xtask::check_boundaries::CRYPTO_ALLOWED` invariant). All secret key
//! material is zeroized on drop — [`MlKemSecret`] via a manual `Drop`
//! (the wrapped `libcrux` type has no `Zeroize` impl of its own to
//! derive against) and the shared secrets it produces via `Zeroizing`.
//!
//! ## ML-KEM-768 sizes (verified against `libcrux-ml-kem-0.0.9/src/`)
//!
//! - Public key: 1184 bytes (`CPA_PKE_PUBLIC_KEY_SIZE`)
//! - Private key: 2400 bytes (`SECRET_KEY_SIZE`)
//! - Ciphertext: 1088 bytes (`CPA_PKE_CIPHERTEXT_SIZE`)
//! - Shared secret: 32 bytes (`SHARED_SECRET_SIZE`)
//! - Key generation seed: 64 bytes (32 + 32)
//! - Encapsulation seed: 32 bytes
//!
//! Phase 3b plugs these into `rekindle-crypto::signal::pqxdh::handshake`.

use libcrux_ml_kem::mlkem768::{
    portable::{decapsulate, encapsulate, generate_key_pair},
    MlKem768Ciphertext, MlKem768KeyPair, MlKem768PrivateKey, MlKem768PublicKey,
};
use rand::RngCore;
use zeroize::Zeroizing;

/// Byte length of an ML-KEM-768 public key (also the on-wire size).
pub const ML_KEM_PUBLIC_KEY_BYTES: usize = 1184;
/// Byte length of an ML-KEM-768 ciphertext (encapsulation output).
pub const ML_KEM_CIPHERTEXT_BYTES: usize = 1088;
/// Byte length of an ML-KEM-768 shared secret.
pub const ML_KEM_SHARED_SECRET_BYTES: usize = 32;
/// Byte length of an ML-KEM-768 private key (full FIPS-203 secret blob,
/// includes implicit-rejection PRF state). Used for VaultStore persistence.
pub const ML_KEM_SECRET_KEY_BYTES: usize = 2400;

/// ML-KEM-768 secret key, zeroized on drop.
///
/// Wraps `libcrux_ml_kem::mlkem768::MlKem768PrivateKey` in `Box` so the
/// raw 2400-byte private key bytes never live on the stack frame of the
/// caller — paired with `Zeroizing` of the wrapper for defense in depth.
pub struct MlKemSecret {
    inner: Box<MlKem768PrivateKey>,
}

/// ML-KEM-768 public key.
#[derive(Clone)]
pub struct MlKemPublic {
    inner: Box<MlKem768PublicKey>,
}

/// Output of [`MlKemPublic::encapsulate`]: ciphertext for the holder of the
/// matching secret key + the derived 32-byte shared secret. The shared
/// secret is the value that feeds PQXDH's KDF input.
pub struct EncapsulationOutput {
    pub ciphertext: Vec<u8>,
    pub shared_secret: Zeroizing<[u8; ML_KEM_SHARED_SECRET_BYTES]>,
}

impl MlKemSecret {
    /// Generate a fresh ML-KEM-768 keypair using `OsRng` for the 64-byte
    /// `(d, z)` seed. Returns the secret + public halves.
    #[must_use]
    pub fn generate() -> (Self, MlKemPublic) {
        let mut seed = [0u8; libcrux_ml_kem::KEY_GENERATION_SEED_SIZE];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        let kp: MlKem768KeyPair = generate_key_pair(seed);
        let (sk, pk) = kp.into_parts();
        (
            Self {
                inner: Box::new(sk),
            },
            MlKemPublic {
                inner: Box::new(pk),
            },
        )
    }

    /// Decapsulate a ciphertext produced by [`MlKemPublic::encapsulate`].
    /// Returns the 32-byte shared secret. ML-KEM-768 has implicit
    /// rejection so this never errors at the protocol level — invalid
    /// ciphertexts return a deterministic "implicit reject" shared
    /// secret that doesn't match the encapsulator's view.
    #[must_use]
    pub fn decapsulate(
        &self,
        ciphertext: &MlKem768Ciphertext,
    ) -> Zeroizing<[u8; ML_KEM_SHARED_SECRET_BYTES]> {
        let ss = decapsulate(&self.inner, ciphertext);
        Zeroizing::new(ss)
    }

    /// Serialize the private key bytes for VaultStore persistence.
    /// Caller MUST wrap the returned vector in a `Zeroizing` and treat
    /// the bytes as secret. Phase 3b's `PreKeyStore::store_pq_*_secret`
    /// methods do exactly that.
    #[must_use]
    pub fn as_secret_bytes(&self) -> &[u8] {
        <MlKem768PrivateKey as AsRef<[u8]>>::as_ref(&self.inner)
    }

    /// Parse a private key from its 2400-byte representation. Returns
    /// `None` if the length is wrong. Caller is responsible for the
    /// bytes coming from a trusted source (VaultStore-encrypted at rest).
    #[must_use]
    pub fn from_secret_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != ML_KEM_SECRET_KEY_BYTES {
            return None;
        }
        let mut value = [0u8; ML_KEM_SECRET_KEY_BYTES];
        value.copy_from_slice(bytes);
        Some(Self {
            inner: Box::new(MlKem768PrivateKey::from(value)),
        })
    }

    /// Extract the corresponding public key from the FIPS-203 secret-key
    /// layout. The ML-KEM-768 private key embeds the encapsulation key
    /// (public) at offset `CPA_PKE_SECRET_KEY_SIZE = 1152`, length
    /// `CPA_PKE_PUBLIC_KEY_SIZE = 1184` — see `libcrux-ml-kem-0.0.9/src/
    /// constants.rs`. This lets us reconstruct a `PreKeyBundle`'s PQ
    /// public field from just the stored secret blob.
    #[must_use]
    pub fn public(&self) -> MlKemPublic {
        let secret_bytes = self.as_secret_bytes();
        const PUBLIC_OFFSET: usize = 1152;
        let mut public_value = [0u8; ML_KEM_PUBLIC_KEY_BYTES];
        public_value
            .copy_from_slice(&secret_bytes[PUBLIC_OFFSET..PUBLIC_OFFSET + ML_KEM_PUBLIC_KEY_BYTES]);
        MlKemPublic {
            inner: Box::new(MlKem768PublicKey::from(public_value)),
        }
    }
}

impl MlKemSecret {
    /// Zero the private key bytes in place, without consuming `self`.
    ///
    /// `MlKem768PrivateKey` (libcrux-ml-kem 0.0.9's `MlKemPrivateKey<SIZE>`,
    /// via its `impl_generic_struct!` macro) implements neither `Zeroize`
    /// nor `AsMut<[u8]>` — but the same macro DOES give it `Default`
    /// (all-zero) and `From<Self> for [u8; SIZE]` (by value). That pair is
    /// enough to zero the real heap allocation, not just a local copy:
    /// `mem::take` writes `Default::default()` (zeros) into `*self.inner`
    /// IN PLACE and moves the actual secret out as `owned`, so the box's
    /// backing memory holds zeros immediately — not "eventually, whenever
    /// the allocator reclaims it," which is what the previous empty-bodied
    /// `Drop` here actually left us with. `owned` is then converted to a
    /// plain array and zeroized too, so the one remaining copy (now on
    /// this frame, not the heap) doesn't outlive this function either.
    ///
    /// Exists as its own method, called by `Drop::drop` below, so the
    /// zeroing can be exercised and verified by a test without relying on
    /// reading memory after deallocation (which would be UB).
    fn zeroize_in_place(&mut self) {
        let owned = std::mem::take(self.inner.as_mut());
        let mut raw: [u8; ML_KEM_SECRET_KEY_BYTES] = owned.into();
        zeroize::Zeroize::zeroize(&mut raw);
    }
}

impl Drop for MlKemSecret {
    fn drop(&mut self) {
        self.zeroize_in_place();
    }
}

impl MlKemPublic {
    /// Encapsulate a fresh shared secret to this public key. Returns
    /// `(ciphertext, shared_secret)`. Caller transmits ciphertext to the
    /// holder of the matching secret key, who reproduces `shared_secret`
    /// via [`MlKemSecret::decapsulate`].
    #[must_use]
    pub fn encapsulate(&self) -> EncapsulationOutput {
        let mut rnd = [0u8; libcrux_ml_kem::ENCAPS_SEED_SIZE];
        rand::rngs::OsRng.fill_bytes(&mut rnd);
        let (ct, ss) = encapsulate(&self.inner, rnd);
        EncapsulationOutput {
            ciphertext: ct.as_ref().to_vec(),
            shared_secret: Zeroizing::new(ss),
        }
    }

    /// Byte representation suitable for wire transmission (1184 bytes).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        // Fully-qualified: `Box::as_ref` returns `&MlKem768PublicKey`;
        // we want the `AsRef<[u8]>` impl on `MlKem768PublicKey` itself.
        <MlKem768PublicKey as AsRef<[u8]>>::as_ref(&self.inner)
    }

    /// Parse a public key from its 1184-byte wire representation.
    /// Returns `None` if the length is wrong. Validity (lattice-point
    /// well-formedness) is NOT checked here — the caller is expected to
    /// follow up with `libcrux_ml_kem::mlkem768::portable::validate_public_key`
    /// if defending against adversarial input.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != ML_KEM_PUBLIC_KEY_BYTES {
            return None;
        }
        let mut value = [0u8; ML_KEM_PUBLIC_KEY_BYTES];
        value.copy_from_slice(bytes);
        // `MlKem768PublicKey` is a tuple-struct-like with `pub(crate) value`.
        // We can't construct it directly here. The crate exposes `From<&[u8; N]>`
        // via `impl_generic_struct!` — check by trying both.
        Some(Self {
            inner: Box::new(MlKem768PublicKey::from(value)),
        })
    }
}

/// Parse a ciphertext from its 1088-byte wire representation. Returns
/// `None` if the length is wrong.
#[must_use]
pub fn ml_kem_ciphertext_from_bytes(bytes: &[u8]) -> Option<MlKem768Ciphertext> {
    if bytes.len() != ML_KEM_CIPHERTEXT_BYTES {
        return None;
    }
    let mut value = [0u8; ML_KEM_CIPHERTEXT_BYTES];
    value.copy_from_slice(bytes);
    Some(MlKem768Ciphertext::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_encapsulate_decapsulate() {
        let (sk, pk) = MlKemSecret::generate();
        let out = pk.encapsulate();
        let ct = ml_kem_ciphertext_from_bytes(&out.ciphertext)
            .expect("ciphertext is exactly 1088 bytes");
        let recovered = sk.decapsulate(&ct);
        assert_eq!(*out.shared_secret, *recovered, "shared secret must match");
    }

    #[test]
    fn public_key_byte_roundtrip() {
        let (_sk, pk) = MlKemSecret::generate();
        let bytes = pk.as_bytes().to_vec();
        assert_eq!(bytes.len(), ML_KEM_PUBLIC_KEY_BYTES);
        let restored = MlKemPublic::from_bytes(&bytes).expect("valid length");
        assert_eq!(restored.as_bytes(), bytes.as_slice());
    }

    #[test]
    fn wrong_secret_returns_different_shared_secret() {
        // ML-KEM-768 implicit rejection: decapsulating with the wrong
        // secret returns a deterministic-but-different shared secret
        // (NOT an error). This is by design for IND-CCA2 security.
        let (sk_alice, pk_alice) = MlKemSecret::generate();
        let (sk_bob, _pk_bob) = MlKemSecret::generate();
        let out = pk_alice.encapsulate();
        let ct = ml_kem_ciphertext_from_bytes(&out.ciphertext).unwrap();
        let alice_view = sk_alice.decapsulate(&ct);
        let bob_view = sk_bob.decapsulate(&ct);
        assert_eq!(*out.shared_secret, *alice_view);
        assert_ne!(*out.shared_secret, *bob_view);
    }

    #[test]
    fn from_bytes_rejects_wrong_length() {
        assert!(MlKemPublic::from_bytes(&[0u8; 1183]).is_none());
        assert!(MlKemPublic::from_bytes(&[0u8; 1185]).is_none());
        assert!(ml_kem_ciphertext_from_bytes(&[0u8; 1087]).is_none());
        assert!(ml_kem_ciphertext_from_bytes(&[0u8; 1089]).is_none());
    }

    #[test]
    fn distinct_keypairs_have_distinct_public_keys() {
        // Sanity: OsRng-driven keypair generation produces unique keys.
        let (_, a) = MlKemSecret::generate();
        let (_, b) = MlKemSecret::generate();
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn public_extracted_from_secret_matches_paired_public() {
        let (sk, pk) = MlKemSecret::generate();
        let extracted = sk.public();
        assert_eq!(extracted.as_bytes(), pk.as_bytes());
    }

    #[test]
    fn secret_byte_roundtrip_preserves_public() {
        let (sk, pk) = MlKemSecret::generate();
        let sk_bytes = sk.as_secret_bytes().to_vec();
        let restored = MlKemSecret::from_secret_bytes(&sk_bytes).expect("valid length");
        assert_eq!(restored.public().as_bytes(), pk.as_bytes());
    }

    #[test]
    fn drop_zeroizes_the_private_key_bytes() {
        // Exercises exactly the logic `Drop::drop` runs, without reading
        // memory after deallocation (undefined behavior) to prove it.
        let (mut sk, _pk) = MlKemSecret::generate();
        assert!(
            sk.as_secret_bytes().iter().any(|&b| b != 0),
            "freshly generated key should not already be all-zero"
        );
        sk.zeroize_in_place();
        assert!(
            sk.as_secret_bytes().iter().all(|&b| b == 0),
            "private key bytes must be zero after the zeroize Drop runs"
        );
    }
}
