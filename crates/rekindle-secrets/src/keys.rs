//! Secret key wrapper types with Zeroize guarantees.
//!
//! Every secret in Rekindle passes through one of these types.
//! All implement `ZeroizeOnDrop` — memory is scrubbed when the value drops.

use aes_gcm::{
    aead::{generic_array::GenericArray, Aead, AeadInPlace, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use rekindle_types::error::CryptoError;
use rekindle_utils::random::RandomSource;
use zeroize::ZeroizeOnDrop;

/// A user's master secret — the root of all identity derivation.
/// Stored in Stronghold, never leaves the device.
#[derive(Clone, ZeroizeOnDrop)]
pub struct MasterSecret(pub [u8; 32]);

/// Shared secret used to derive SMPL slot keypairs for a community.
/// Distributed to all members via InviteSecrets.
#[derive(Clone, ZeroizeOnDrop)]
pub struct SlotSeed(pub [u8; 32]);

impl SlotSeed {
    /// A random slot seed from `rng`, the engine's injected generator.
    #[must_use]
    pub fn generate_with(rng: &dyn RandomSource) -> Self {
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes);
        Self(bytes)
    }
}

/// The channel-body AAD (plan E3.3): `domains::CHANNEL_BODY_AAD ‖
/// u32be(len) anchor ‖ u32be(len) channel_id ‖ author ‖ stream_seq_le64`.
///
/// Binds a body to its community, channel (or thread), author and the
/// author's stream position, not to a slot: a body lifted to another
/// channel, author or position does not open. The two text fields are
/// length-prefixed so no two positions share bytes.
#[derive(Debug, Clone, Copy)]
pub struct ChannelAad<'a> {
    pub anchor: &'a str,
    pub channel_id: &'a str,
    pub author: &'a [u8; 32],
    pub stream_seq: u64,
}

impl ChannelAad<'_> {
    /// Encode to the canonical bytes consumed by AES-GCM.
    #[must_use]
    pub fn to_bytes(self) -> Vec<u8> {
        let domain = rekindle_types::domains::CHANNEL_BODY_AAD.as_bytes();
        let mut out =
            Vec::with_capacity(domain.len() + 8 + self.anchor.len() + self.channel_id.len() + 40);
        out.extend_from_slice(domain);
        for text in [self.anchor, self.channel_id] {
            out.extend_from_slice(&u32::try_from(text.len()).unwrap_or(u32::MAX).to_be_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        out.extend_from_slice(self.author);
        out.extend_from_slice(&self.stream_seq.to_le_bytes());
        out
    }
}

/// Media Encryption Key for group/channel message encryption.
///
/// Each channel has its own MEK. Rotated on membership changes via the
/// deterministic rotator protocol (peer-to-peer, no coordinator).
#[derive(Clone, ZeroizeOnDrop)]
pub struct MediaEncryptionKey {
    key: [u8; 32],
    /// Monotonically increasing generation number for rotation tracking.
    #[zeroize(skip)]
    generation: u64,
    /// Pseudonym of the rotator that minted this key. `None` for keys
    /// constructed internally or received from a peer that predates
    /// provenance.
    #[zeroize(skip)]
    rotator_pseudonym: Option<[u8; 32]>,
    /// The minter's deterministic election rank —
    /// `blake3(election_context || minter_pseudonym)`, the same value
    /// `rotator::select_rotator` ranks by.
    ///
    /// This is what lets two peers who minted *different* key bytes at
    /// the *same* generation converge:
    /// `rekindle_mek_rotation::convergence::incoming_wins_same_generation`
    /// keeps the lower rank, and every peer computes the same answer
    /// regardless of arrival order. Without it the two keys are
    /// indistinguishable and the community splits into halves that
    /// cannot read each other.
    #[zeroize(skip)]
    election_rank: Option<[u8; 32]>,
}

impl MediaEncryptionKey {
    /// Generate a new random MEK at the given generation.
    pub fn generate(generation: u64) -> Self {
        Self::generate_with(generation, &rekindle_utils::random::OsRandom)
    }

    /// A new random MEK at `generation`, from `rng`, the engine's injected
    /// generator (plan C9.2).
    #[must_use]
    pub fn generate_with(generation: u64, rng: &dyn RandomSource) -> Self {
        let mut key = [0u8; 32];
        rng.fill(&mut key);
        Self {
            key,
            generation,
            rotator_pseudonym: None,
            election_rank: None,
        }
    }

    /// Restore a MEK from raw bytes and generation.
    pub fn from_bytes(key: [u8; 32], generation: u64) -> Self {
        Self {
            key,
            generation,
            rotator_pseudonym: None,
            election_rank: None,
        }
    }

    /// Stamp the minter's identity and election rank onto a freshly
    /// generated key. See [`Self::election_rank`] for why this matters.
    #[must_use]
    pub fn with_provenance(mut self, rotator_pseudonym: [u8; 32], election_rank: [u8; 32]) -> Self {
        self.rotator_pseudonym = Some(rotator_pseudonym);
        self.election_rank = Some(election_rank);
        self
    }

    /// The minting rotator's pseudonym, if recorded.
    #[must_use]
    pub fn rotator_pseudonym(&self) -> Option<[u8; 32]> {
        self.rotator_pseudonym
    }

    /// The minter's election rank, if recorded.
    #[must_use]
    pub fn election_rank(&self) -> Option<[u8; 32]> {
        self.election_rank
    }

    /// Get the raw key bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }

    /// Get the generation number.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Encrypt plaintext with AES-256-GCM.
    ///
    /// Output format: `[12-byte nonce || ciphertext + 16-byte tag]`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::Encryption(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::Encryption(e.to_string()))?;

        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt ciphertext (expects `[12-byte nonce || ciphertext + tag]`).
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 12 {
            return Err(CryptoError::Decryption("data too short for nonce".into()));
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::Decryption(e.to_string()))?;

        let nonce = Nonce::from_slice(&data[..12]);
        cipher
            .decrypt(nonce, &data[12..])
            .map_err(|e| CryptoError::Decryption(e.to_string()))
    }

    /// Serialize to the wire format.
    ///
    /// Base 40 bytes: `[generation LE(8) || key(32)]`. When provenance
    /// is present a 65-byte suffix follows:
    /// `[flag(1)=1 || rotator_pseudonym(32) || election_rank(32)]`, for
    /// 105 total. The suffix is append-only so a reader that stops at 40
    /// still gets a valid key — which is what made rolling this out
    /// possible without a flag day.
    ///
    /// Used for both keystore persistence and network transport (inside
    /// the per-recipient `wrap_mek` blob), so provenance round-trips
    /// through both.
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(40);
        buf.extend_from_slice(&self.generation.to_le_bytes());
        buf.extend_from_slice(&self.key);
        if let (Some(pseudonym), Some(rank)) = (self.rotator_pseudonym, self.election_rank) {
            buf.reserve(65);
            buf.push(1); // provenance-present flag
            buf.extend_from_slice(&pseudonym);
            buf.extend_from_slice(&rank);
        }
        buf
    }

    /// Deserialize from the wire format. Returns `None` if shorter than
    /// the 40-byte base. A 40-byte blob yields `None` provenance.
    pub fn from_wire_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 {
            return None;
        }
        let generation = u64::from_le_bytes(bytes[..8].try_into().ok()?);
        let key: [u8; 32] = bytes[8..40].try_into().ok()?;
        let (rotator_pseudonym, election_rank) = if bytes.len() >= 105 && bytes[40] == 1 {
            let pseudonym: [u8; 32] = bytes[41..73].try_into().ok()?;
            let rank: [u8; 32] = bytes[73..105].try_into().ok()?;
            (Some(pseudonym), Some(rank))
        } else {
            (None, None)
        };
        Some(Self {
            key,
            generation,
            rotator_pseudonym,
            election_rank,
        })
    }

    /// Encrypt bound to `aad` (a [`ChannelAad`]'s bytes). Output is `nonce(12) || ciphertext+tag`. The receiver must
    /// reconstruct the same AAD or decryption fails.
    pub fn encrypt_with_aad(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::Encryption(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = GenericArray::clone_from_slice(&nonce_bytes);

        let mut buffer = plaintext.to_vec();
        cipher
            .encrypt_in_place(&nonce, aad, &mut buffer)
            .map_err(|e| CryptoError::Encryption(e.to_string()))?;

        let mut output = Vec::with_capacity(12 + buffer.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&buffer);
        Ok(output)
    }

    /// Decrypt a ciphertext produced by [`Self::encrypt_with_aad`].
    pub fn decrypt_with_aad(&self, data: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 12 {
            return Err(CryptoError::Decryption("data too short".into()));
        }
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::Decryption(e.to_string()))?;
        let nonce = Nonce::from_slice(&data[..12]);
        cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &data[12..],
                    aad,
                },
            )
            .map_err(|e| CryptoError::Decryption(e.to_string()))
    }
}

/// Ephemeral symmetric key for a direct or group voice/video call.
#[derive(Clone, ZeroizeOnDrop)]
pub struct CallKey(pub [u8; 32]);

impl CallKey {
    /// Generate a random call key.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
