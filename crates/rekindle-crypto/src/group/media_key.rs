use aes_gcm::{
    aead::{generic_array::GenericArray, Aead, AeadInPlace, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;

/// Architecture §8 line 1626 — channel-message AAD format:
/// `channel_record_key || subkey_index_le32 || lamport_ts_le64`.
/// Binds the ciphertext to its channel + position so an attacker
/// can't replay a ciphertext into a different channel or at a
/// different position in the message stream.
#[derive(Debug, Clone, Copy)]
pub struct ChannelAad<'a> {
    pub channel_record_key: &'a [u8],
    pub subkey_index: u32,
    pub lamport_ts: u64,
}

impl ChannelAad<'_> {
    /// Encode to the canonical wire bytes consumed by AES-GCM.
    pub fn to_bytes(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.channel_record_key.len() + 4 + 8);
        out.extend_from_slice(self.channel_record_key);
        out.extend_from_slice(&self.subkey_index.to_le_bytes());
        out.extend_from_slice(&self.lamport_ts.to_le_bytes());
        out
    }
}

/// Media Encryption Key for group/channel message encryption.
///
/// Each community channel has its own MEK. It's distributed to members
/// via their individual Signal sessions and rotated on membership changes.
#[derive(Clone, ZeroizeOnDrop)]
pub struct MediaEncryptionKey {
    key: [u8; 32],
    /// Monotonically increasing generation number for key rotation tracking.
    #[zeroize(skip)]
    generation: u64,
    /// Pseudonym of the rotator that minted this key (provenance). `None`
    /// for legacy/internally-constructed keys with no recorded minter.
    #[zeroize(skip)]
    rotator_pseudonym: Option<[u8; 32]>,
    /// The minter's deterministic election rank — `blake3(election_context ||
    /// minter_pseudonym)`, the same value `cascade_candidates`/`select_rotator`
    /// rank by (see `rekindle-secrets::rotator`). Used to deterministically
    /// resolve same-generation key collisions (split-brain): the key whose
    /// rank is lexicographically lowest (= the rightful primary rotator) wins
    /// on every peer. `None` for legacy/untagged keys, which lose to any
    /// tagged key.
    #[zeroize(skip)]
    election_rank: Option<[u8; 32]>,
}

impl MediaEncryptionKey {
    /// Generate a new random MEK.
    pub fn generate(generation: u64) -> Self {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self {
            key,
            generation,
            rotator_pseudonym: None,
            election_rank: None,
        }
    }

    /// Restore a MEK from raw bytes.
    pub fn from_bytes(key: [u8; 32], generation: u64) -> Self {
        Self {
            key,
            generation,
            rotator_pseudonym: None,
            election_rank: None,
        }
    }

    /// Attach minter provenance (rotator pseudonym + election rank) to a
    /// freshly-minted key so receivers can deterministically converge on the
    /// canonical key for a generation. See [`Self::election_rank`].
    #[must_use]
    pub fn with_provenance(mut self, rotator_pseudonym: [u8; 32], election_rank: [u8; 32]) -> Self {
        self.rotator_pseudonym = Some(rotator_pseudonym);
        self.election_rank = Some(election_rank);
        self
    }

    /// The minting rotator's pseudonym, if recorded.
    pub fn rotator_pseudonym(&self) -> Option<[u8; 32]> {
        self.rotator_pseudonym
    }

    /// The minter's deterministic election rank, if recorded. Lower wins when
    /// resolving same-generation collisions.
    pub fn election_rank(&self) -> Option<[u8; 32]> {
        self.election_rank
    }

    /// Get the raw key bytes (for encrypted distribution to members).
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }

    /// Get the key generation number.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Encrypt a plaintext message with no associated data. Use only
    /// for non-channel payloads where AAD binding doesn't apply (MEK
    /// distribution wraps, voice frames keyed by stream-id, etc.).
    /// Channel chat messages MUST use [`Self::encrypt_with_aad`].
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        // Prepend nonce to ciphertext
        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Architecture §8 line 1626 — encrypt with the canonical channel
    /// AAD. Output is `nonce(12) || ciphertext+tag`. Receiver must
    /// reconstruct the same AAD or decryption fails — preventing
    /// cross-channel and replay-position ciphertext attacks.
    pub fn encrypt_with_aad(
        &self,
        plaintext: &[u8],
        aad: ChannelAad<'_>,
    ) -> Result<Vec<u8>, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = GenericArray::clone_from_slice(&nonce_bytes);

        let aad_bytes = aad.to_bytes();
        let mut buffer = plaintext.to_vec();
        cipher
            .encrypt_in_place(&nonce, &aad_bytes, &mut buffer)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        let mut output = Vec::with_capacity(12 + buffer.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&buffer);
        Ok(output)
    }

    /// Serialize to the wire format. Base 40 bytes: generation (8 LE) + key
    /// (32). When provenance is present, append 65 bytes:
    /// `flag(1)=1 || rotator_pseudonym(32) || election_rank(32)` → 105 bytes.
    /// Backward-compatible: a reader on the old build sees only the first 40.
    ///
    /// Used for Stronghold persistence and network transport (inside the
    /// per-recipient `wrap_mek` blob), so provenance round-trips both.
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

    /// Deserialize from the wire format. Returns `None` if too short. Parses
    /// the optional 65-byte provenance suffix when present (105-byte form);
    /// a 40-byte (legacy) blob deserializes with `None` provenance.
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

    /// Decrypt a ciphertext message (expects nonce prepended). Use
    /// only for the no-AAD path; channel messages must call
    /// [`Self::decrypt_with_aad`] with the matching `ChannelAad`.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 12 {
            return Err(CryptoError::DecryptionError("data too short".into()));
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))?;

        let nonce = Nonce::from_slice(&data[..12]);
        let ciphertext = &data[12..];

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))
    }

    /// Architecture §8 line 1626 — decrypt with the canonical channel
    /// AAD. Tag verification fails when the receiver reconstructs a
    /// different AAD, blocking cross-channel replay attacks.
    pub fn decrypt_with_aad(
        &self,
        data: &[u8],
        aad: ChannelAad<'_>,
    ) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 12 {
            return Err(CryptoError::DecryptionError("data too short".into()));
        }
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))?;
        let nonce = Nonce::from_slice(&data[..12]);
        let aad_bytes = aad.to_bytes();
        cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &data[12..],
                    aad: &aad_bytes,
                },
            )
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let mek = MediaEncryptionKey::generate(1);
        let plaintext = b"hello from a community channel";

        let encrypted = mek.encrypt(plaintext).unwrap();
        let decrypted = mek.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext.as_slice(), &decrypted);
    }

    #[test]
    fn wire_bytes_roundtrip() {
        let mek = MediaEncryptionKey::generate(42);
        let wire = mek.to_wire_bytes();
        assert_eq!(wire.len(), 40);

        let restored = MediaEncryptionKey::from_wire_bytes(&wire).unwrap();
        assert_eq!(restored.generation(), 42);
        assert_eq!(restored.as_bytes(), mek.as_bytes());
    }

    #[test]
    fn wire_bytes_too_short() {
        assert!(MediaEncryptionKey::from_wire_bytes(&[0u8; 39]).is_none());
        assert!(MediaEncryptionKey::from_wire_bytes(&[]).is_none());
    }

    #[test]
    fn legacy_wire_bytes_have_no_provenance() {
        let mek = MediaEncryptionKey::generate(7);
        let wire = mek.to_wire_bytes();
        assert_eq!(wire.len(), 40, "untagged key stays 40 bytes (backward-compatible)");
        let restored = MediaEncryptionKey::from_wire_bytes(&wire).unwrap();
        assert_eq!(restored.rotator_pseudonym(), None);
        assert_eq!(restored.election_rank(), None);
    }

    #[test]
    fn provenance_wire_roundtrip() {
        let pseudonym = [9u8; 32];
        let rank = [3u8; 32];
        let mek = MediaEncryptionKey::generate(11).with_provenance(pseudonym, rank);
        let wire = mek.to_wire_bytes();
        assert_eq!(wire.len(), 105, "tagged key is 40 + 1 + 32 + 32");

        let restored = MediaEncryptionKey::from_wire_bytes(&wire).unwrap();
        assert_eq!(restored.generation(), 11);
        assert_eq!(restored.as_bytes(), mek.as_bytes());
        assert_eq!(restored.rotator_pseudonym(), Some(pseudonym));
        assert_eq!(restored.election_rank(), Some(rank));
    }

    #[test]
    fn tagged_key_decodes_on_old_reader_prefix() {
        // An old build reads only the first 40 bytes; the suffix is ignored
        // and the key/generation still decode correctly.
        let mek = MediaEncryptionKey::generate(5).with_provenance([1u8; 32], [2u8; 32]);
        let wire = mek.to_wire_bytes();
        let legacy_view = MediaEncryptionKey::from_wire_bytes(&wire[..40]).unwrap();
        assert_eq!(legacy_view.generation(), 5);
        assert_eq!(legacy_view.as_bytes(), mek.as_bytes());
        assert_eq!(legacy_view.election_rank(), None);
    }

    #[test]
    fn different_keys_fail() {
        let mek1 = MediaEncryptionKey::generate(1);
        let mek2 = MediaEncryptionKey::generate(2);
        let plaintext = b"secret message";

        let encrypted = mek1.encrypt(plaintext).unwrap();
        assert!(mek2.decrypt(&encrypted).is_err());
    }
}
