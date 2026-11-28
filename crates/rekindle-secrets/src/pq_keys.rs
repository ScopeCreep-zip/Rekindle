//! ML-KEM-768 wrappers — post-quantum key encapsulation primitives.
//!
//! Phase 3a of the decomposed-harvest plan. Sole consumer of
//! [`libcrux_ml_kem::mlkem768`] in the workspace (per the
//! `xtask::check_boundaries::CRYPTO_ALLOWED` invariant). All secret key
//! material is zeroized on drop — [`MlKemSecret`] via a manual `Drop`
//! (the wrapped `libcrux` type has no `Zeroize` impl of its own to
//! derive against) and the shared secrets it produces via `Zeroizing`.
//!
//! ## ML-KEM-768 sizes (verified against `libcrux-ml-kem-0.0.10/src/`)
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
    portable::{decapsulate, encapsulate, generate_key_pair, validate_public_key},
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
/// ML-KEM key generation's seed `d ‖ z` (FIPS 203 §7.1), 64 bytes.
pub const ML_KEM_KEYGEN_SEED_BYTES: usize = libcrux_ml_kem::KEY_GENERATION_SEED_SIZE;

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
        let mut seed = zeroize::Zeroizing::new([0u8; ML_KEM_KEYGEN_SEED_BYTES]);
        rand::rngs::OsRng.fill_bytes(&mut *seed);
        Self::from_seed(&seed)
    }

    /// The keypair FIPS 203's `ML-KEM.KeyGen_internal(d, z)` derives from a
    /// 64-byte seed `d ‖ z`: the caller's generator supplies it (the
    /// Host's, so a seeded simulation mints the same keys; plan E2.4).
    #[must_use]
    pub fn from_seed(seed: &[u8; ML_KEM_KEYGEN_SEED_BYTES]) -> (Self, MlKemPublic) {
        let kp: MlKem768KeyPair = generate_key_pair(*seed);
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
    /// bytes coming from a trusted source (the identity database).
    ///
    /// The staging copy is `Zeroizing`, and the key is written into a
    /// box allocated first, so no unwiped copy of the secret is left on
    /// this frame or in a freed allocation.
    #[must_use]
    pub fn from_secret_bytes(bytes: &[u8]) -> Option<Self> {
        let value: Zeroizing<[u8; ML_KEM_SECRET_KEY_BYTES]> =
            Zeroizing::new(bytes.try_into().ok()?);
        let mut inner = Box::<MlKem768PrivateKey>::default();
        *inner = MlKem768PrivateKey::from(&*value);
        Some(Self { inner })
    }

    /// Extract the corresponding public key from the FIPS-203 secret-key
    /// layout. The ML-KEM-768 private key embeds the encapsulation key
    /// (public) at offset `CPA_PKE_SECRET_KEY_SIZE = 1152`, length
    /// `CPA_PKE_PUBLIC_KEY_SIZE = 1184` — see `libcrux-ml-kem-0.0.10/src/
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
    /// `MlKem768PrivateKey` (libcrux-ml-kem 0.0.10's `MlKemPrivateKey<SIZE>`,
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

// The manual Drop implementation wipes the private key in place.
impl zeroize::ZeroizeOnDrop for MlKemSecret {}

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
        // The encapsulation seed `m` determines the shared secret (FIPS
        // 203 §6.2), so it is wiped like one.
        let mut rnd = Zeroizing::new([0u8; libcrux_ml_kem::ENCAPS_SEED_SIZE]);
        rand::rngs::OsRng.fill_bytes(&mut *rnd);
        self.encapsulate_with_seed(&rnd)
    }

    /// FIPS 203's `ML-KEM.Encaps_internal(ek, m)` (§6.2) with the caller's
    /// 32-byte `m`. Private: production draws `m` from `OsRng` in
    /// [`Self::encapsulate`]; the known-answer tests (`pq_keys_kat.rs`)
    /// pass NIST's `m` through the same path.
    fn encapsulate_with_seed(
        &self,
        m: &Zeroizing<[u8; libcrux_ml_kem::ENCAPS_SEED_SIZE]>,
    ) -> EncapsulationOutput {
        let (ct, ss) = encapsulate(&self.inner, **m);
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
    /// Returns `None` if the length is wrong or the key fails FIPS 203
    /// §7.2's encapsulation-key check (every coefficient of `t` reduced
    /// mod q: `ByteEncode12(ByteDecode12(ek)) == ek`), via libcrux's
    /// `validate_public_key`. Every key from a peer comes through here,
    /// so none is encapsulated to unchecked.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let value: &[u8; ML_KEM_PUBLIC_KEY_BYTES] = bytes.try_into().ok()?;
        let inner = Box::new(MlKem768PublicKey::from(value));
        validate_public_key(&inner).then_some(Self { inner })
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
#[path = "pq_keys_kat.rs"]
mod kat;

#[cfg(test)]
#[path = "pq_keys_tests.rs"]
mod tests;
