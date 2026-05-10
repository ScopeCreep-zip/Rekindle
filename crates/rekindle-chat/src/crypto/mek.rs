//! MEK (Media Encryption Key) in-memory cache backed by vault persistence.
//!
//! Also provides MEK-level AES-256-GCM encrypt/decrypt for channel messages,
//! and ECDH-based wrap/unwrap for per-member MEK distribution during community
//! create, join, and key rotation.

use std::collections::HashMap;
use std::sync::Arc;

use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use aws_lc_rs::rand::SecureRandom;
use parking_lot::RwLock;
use zeroize::Zeroizing;

use rekindle_storage::VaultStore;

use crate::ChatError;

/// Snapshot of a single cached MEK entry for display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MekSnapshot {
    pub channel_id: String,
    pub generation: u64,
}

struct CachedMek {
    key: [u8; 32],
    generation: u64,
}

pub struct MekCache {
    entries: RwLock<HashMap<(String, String), Vec<CachedMek>>>,
    vault: Arc<VaultStore>,
}

impl MekCache {
    /// Load all cached MEKs from vault on unlock.
    pub fn from_vault(vault: Arc<VaultStore>) -> Result<Self, ChatError> {
        let mut entries: HashMap<(String, String), Vec<CachedMek>> = HashMap::new();
        for e in vault.load_all_meks()? {
            let key = (e.community_id.clone(), e.channel_id.clone());
            entries
                .entry(key)
                .or_default()
                .push(CachedMek {
                    key: e.key_bytes,
                    generation: e.generation,
                });
        }
        // Sort each channel's MEKs by generation
        for v in entries.values_mut() {
            v.sort_by_key(|m| m.generation);
        }
        Ok(Self {
            entries: RwLock::new(entries),
            vault,
        })
    }

    /// Get the current (highest generation) MEK for a channel.
    pub fn current(&self, community: &str, channel: &str) -> Option<([u8; 32], u64)> {
        let entries = self.entries.read();
        entries
            .get(&(community.to_string(), channel.to_string()))
            .and_then(|v| v.last())
            .map(|m| (m.key, m.generation))
    }

    /// Get a specific generation MEK.
    pub fn get_generation(
        &self,
        community: &str,
        channel: &str,
        generation: u64,
    ) -> Option<[u8; 32]> {
        let entries = self.entries.read();
        entries
            .get(&(community.to_string(), channel.to_string()))
            .and_then(|v| v.iter().find(|m| m.generation == generation))
            .map(|m| m.key)
    }

    /// Insert a new MEK and persist to vault.
    pub fn insert(
        &self,
        community: &str,
        channel: &str,
        key: [u8; 32],
        generation: u64,
    ) {
        let mut entries = self.entries.write();
        let k = (community.to_string(), channel.to_string());
        let vec = entries.entry(k).or_default();
        if !vec.iter().any(|m| m.generation == generation) {
            vec.push(CachedMek { key, generation });
            vec.sort_by_key(|m| m.generation);
        }
        let _ = self.vault.store_mek(community, channel, generation, &key);
    }

    /// Remove all MEKs for a community.
    pub fn remove_community(&self, community: &str) {
        let mut entries = self.entries.write();
        entries.retain(|(cid, _), _| cid != community);
        let _ = self.vault.delete_community_meks(community);
    }

    /// Snapshot of cached MEKs for a community (for diagnostics/display).
    pub fn snapshot(&self, community: &str) -> Vec<MekSnapshot> {
        let entries = self.entries.read();
        let mut result: Vec<MekSnapshot> = entries
            .iter()
            .filter(|((cid, _), _)| cid == community)
            .flat_map(|((_, channel_id), meks)| {
                meks.iter().map(move |m| MekSnapshot {
                    channel_id: channel_id.clone(),
                    generation: m.generation,
                })
            })
            .collect();
        result.sort_by(|a, b| a.channel_id.cmp(&b.channel_id).then(a.generation.cmp(&b.generation)));
        result
    }

    /// Total cached entry count across all communities.
    pub fn total_entries(&self) -> usize {
        self.entries.read().values().map(Vec::len).sum()
    }

    /// Clear all in-memory MEK entries. Called during lock.
    /// Each [u8; 32] key is overwritten by the Vec drop.
    pub fn clear(&self) {
        let mut entries = self.entries.write();
        // Explicitly zero each key before dropping
        for (_, meks) in entries.iter_mut() {
            for mek in meks.iter_mut() {
                mek.key.fill(0);
            }
        }
        entries.clear();
    }
}

// ── MEK-level AES-256-GCM for channel messages ─────────────────────

const TAG_LEN: usize = 16;

/// Encrypt plaintext with a MEK. Output: `[12-byte nonce || ciphertext || 16-byte tag]`.
///
/// Used for channel message encryption. Each message gets a random nonce.
pub fn mek_encrypt(mek_key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, ChatError> {
    let unbound = UnboundKey::new(&AES_256_GCM, mek_key)
        .map_err(|e| ChatError::Internal(format!("MEK key init: {e}")))?;
    let aead = LessSafeKey::new(unbound);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    aws_lc_rs::rand::SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|e| ChatError::Internal(format!("MEK nonce: {e}")))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.to_vec();
    aead.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|e| ChatError::Internal(format!("MEK encrypt: {e}")))?;

    let mut wire = Vec::with_capacity(NONCE_LEN + in_out.len());
    wire.extend_from_slice(&nonce_bytes);
    wire.extend_from_slice(&in_out);
    Ok(wire)
}

/// Decrypt ciphertext with a MEK. Input: `[12-byte nonce || ciphertext || 16-byte tag]`.
///
/// Used for channel message decryption on receive.
pub fn mek_decrypt(mek_key: &[u8; 32], wire: &[u8]) -> Result<Vec<u8>, ChatError> {
    if wire.len() < NONCE_LEN + TAG_LEN {
        return Err(ChatError::Internal(format!(
            "MEK ciphertext too short: {} bytes (min {})",
            wire.len(),
            NONCE_LEN + TAG_LEN,
        )));
    }

    let unbound = UnboundKey::new(&AES_256_GCM, mek_key)
        .map_err(|e| ChatError::Internal(format!("MEK key init: {e}")))?;
    let aead = LessSafeKey::new(unbound);

    let nonce_bytes: [u8; NONCE_LEN] = wire[..NONCE_LEN]
        .try_into()
        .expect("slice len verified above");
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = wire[NONCE_LEN..].to_vec();
    let plaintext = aead
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| ChatError::Internal("MEK decrypt: GCM tag verification failed".into()))?;
    Ok(plaintext.to_vec())
}

// ── ECDH-based MEK wrapping for per-member distribution ─────────────

/// HKDF info label for MEK wrapping key derivation.
const MEK_WRAP_HKDF_INFO: &[u8] = b"rekindle-mek-wrap-v1";

/// Wrap MEK bytes for a specific recipient via X25519 ECDH + HKDF + AES-256-GCM.
///
/// `sender_x25519_seed`: the sender's X25519 DH private key seed (32 bytes).
/// `recipient_x25519_pub`: the recipient's X25519 DH public key (32 bytes).
/// `mek_wire_bytes`: the MEK in wire format `[generation(8 LE) || key(32)]` (40 bytes).
///
/// Output: `[12-byte nonce || ciphertext || 16-byte tag]` (68 bytes for 40-byte input).
pub fn wrap_mek(
    sender_x25519_seed: &[u8; 32],
    recipient_x25519_pub: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>, ChatError> {
    let wrapping_key = ecdh_derive_wrapping_key(sender_x25519_seed, recipient_x25519_pub)?;
    aes_gcm_wrap(&wrapping_key, mek_wire_bytes)
}

/// Unwrap MEK bytes received from a sender via X25519 ECDH + HKDF + AES-256-GCM.
///
/// `recipient_x25519_seed`: our X25519 DH private key seed (32 bytes).
/// `sender_x25519_pub`: the sender's X25519 DH public key (32 bytes).
/// `wrapped`: the AES-256-GCM wrapped blob from `wrap_mek`.
///
/// Returns the original MEK wire bytes `[generation(8 LE) || key(32)]`.
pub fn unwrap_mek(
    recipient_x25519_seed: &[u8; 32],
    sender_x25519_pub: &[u8; 32],
    wrapped: &[u8],
) -> Result<Vec<u8>, ChatError> {
    let wrapping_key = ecdh_derive_wrapping_key(recipient_x25519_seed, sender_x25519_pub)?;
    aes_gcm_unwrap(&wrapping_key, wrapped)
}

/// Serialize a MEK to wire format: `[generation(8 LE) || key(32)]` (40 bytes).
pub fn mek_to_wire(key: &[u8; 32], generation: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(40);
    buf.extend_from_slice(&generation.to_le_bytes());
    buf.extend_from_slice(key);
    buf
}

/// Deserialize a MEK from wire format. Returns `(key, generation)`.
pub fn mek_from_wire(wire: &[u8]) -> Result<([u8; 32], u64), ChatError> {
    if wire.len() < 40 {
        return Err(ChatError::Internal(format!(
            "MEK wire too short: {} bytes (expected 40)", wire.len()
        )));
    }
    let generation = u64::from_le_bytes(
        wire[..8].try_into().expect("8 bytes"),
    );
    let mut key = [0u8; 32];
    key.copy_from_slice(&wire[8..40]);
    Ok((key, generation))
}

/// X25519 ECDH → HKDF-SHA256 → 32-byte wrapping key.
fn ecdh_derive_wrapping_key(
    our_seed: &[u8; 32],
    their_pub: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, ChatError> {
    let our_key = rekindle_ratchet::crypto::dh::reusable_from_seed(our_seed)
        .map_err(|e| ChatError::Internal(format!("X25519 from seed: {e}")))?;
    let shared = rekindle_ratchet::crypto::dh::ratchet_agree(&our_key, their_pub)
        .map_err(|e| ChatError::Internal(format!("X25519 ECDH: {e}")))?;

    // HKDF-SHA256 to derive wrapping key
    let salt = aws_lc_rs::hkdf::Salt::new(aws_lc_rs::hkdf::HKDF_SHA256, &[]);
    let prk = salt.extract(shared.as_ref());

    struct Len32;
    impl aws_lc_rs::hkdf::KeyType for Len32 {
        fn len(&self) -> usize { 32 }
    }

    let okm = prk.expand(&[MEK_WRAP_HKDF_INFO], Len32)
        .map_err(|_| ChatError::Internal("HKDF expand for MEK wrap".into()))?;
    let mut key = Zeroizing::new([0u8; 32]);
    okm.fill(key.as_mut())
        .map_err(|_| ChatError::Internal("HKDF fill for MEK wrap".into()))?;
    Ok(key)
}

/// AES-256-GCM encrypt (for MEK wrapping).
fn aes_gcm_wrap(key: &Zeroizing<[u8; 32]>, plaintext: &[u8]) -> Result<Vec<u8>, ChatError> {
    let unbound = UnboundKey::new(&AES_256_GCM, key.as_ref())
        .map_err(|e| ChatError::Internal(format!("wrap key init: {e}")))?;
    let aead = LessSafeKey::new(unbound);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    aws_lc_rs::rand::SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|e| ChatError::Internal(format!("wrap nonce: {e}")))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.to_vec();
    aead.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|e| ChatError::Internal(format!("wrap seal: {e}")))?;

    let mut wire = Vec::with_capacity(NONCE_LEN + in_out.len());
    wire.extend_from_slice(&nonce_bytes);
    wire.extend_from_slice(&in_out);
    Ok(wire)
}

/// AES-256-GCM decrypt (for MEK unwrapping).
fn aes_gcm_unwrap(key: &Zeroizing<[u8; 32]>, wire: &[u8]) -> Result<Vec<u8>, ChatError> {
    if wire.len() < NONCE_LEN + TAG_LEN {
        return Err(ChatError::Internal("wrapped MEK too short".into()));
    }
    let unbound = UnboundKey::new(&AES_256_GCM, key.as_ref())
        .map_err(|e| ChatError::Internal(format!("unwrap key init: {e}")))?;
    let aead = LessSafeKey::new(unbound);

    let nonce_bytes: [u8; NONCE_LEN] = wire[..NONCE_LEN]
        .try_into()
        .expect("slice len verified");
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = wire[NONCE_LEN..].to_vec();
    let plaintext = aead
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| ChatError::Internal("MEK unwrap: GCM tag failed".into()))?;
    Ok(plaintext.to_vec())
}
