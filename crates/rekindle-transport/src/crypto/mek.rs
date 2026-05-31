//! Unified MEK (Media Encryption Key) resolution and lifecycle.
//!
//! Single cache replacing the dual `mek_cache` + `channel_mek_cache`.
//! Handles generation lookup, wrap/unwrap via ECDH + HKDF + AES-GCM,
//! and delegates Stronghold persistence to `rekindle-secrets`.

use std::collections::HashMap;
use std::time::Instant;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::PublicKey as X25519PublicKey;
use zeroize::ZeroizeOnDrop;

use crate::error::{Result, TransportError};

/// HKDF info label for MEK wrapping key derivation.
const MEK_WRAP_HKDF_INFO: &[u8] = b"rekindle-mek-wrap-v1";

/// A media encryption key with its generation number.
#[derive(Clone, ZeroizeOnDrop)]
pub struct Mek {
    key: [u8; 32],
    #[zeroize(skip)]
    generation: u64,
}

impl Mek {
    /// Generate a new random MEK at the given generation.
    pub fn generate(generation: u64) -> Self {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self { key, generation }
    }

    /// Restore from raw bytes and generation.
    pub fn from_bytes(key: [u8; 32], generation: u64) -> Self {
        Self { key, generation }
    }

    /// Deserialize from 40-byte wire format: `[generation(8 LE) || key(32)]`.
    pub fn from_wire_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 {
            return None;
        }
        let generation = u64::from_le_bytes(bytes[..8].try_into().ok()?);
        let key: [u8; 32] = bytes[8..40].try_into().ok()?;
        Some(Self { key, generation })
    }

    /// Serialize to 40-byte wire format.
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(40);
        buf.extend_from_slice(&self.generation.to_le_bytes());
        buf.extend_from_slice(&self.key);
        buf
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Encrypt plaintext with this MEK. Returns `[12-byte nonce || ciphertext+tag]`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|e| TransportError::EncryptionFailed {
                reason: e.to_string(),
            })?;
        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct =
            cipher
                .encrypt(nonce, plaintext)
                .map_err(|e| TransportError::EncryptionFailed {
                    reason: e.to_string(),
                })?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Decrypt ciphertext. Expects `[12-byte nonce || ciphertext+tag]`.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            return Err(TransportError::DecryptionFailed {
                reason: "data too short".into(),
            });
        }
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|e| TransportError::DecryptionFailed {
                reason: e.to_string(),
            })?;
        let nonce = Nonce::from_slice(&data[..12]);
        cipher
            .decrypt(nonce, &data[12..])
            .map_err(|e| TransportError::DecryptionFailed {
                reason: e.to_string(),
            })
    }
}

// ── MEK wrapping (ECDH + HKDF + AES-GCM) ────────────────────────────

/// Wrap MEK wire bytes for a specific recipient via X25519 ECDH.
///
/// Output: `[12-byte nonce || ciphertext+tag]` (68 bytes for 40-byte input).
pub fn wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_pub: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>> {
    let shared = ecdh_shared_secret(sender_signing_key, recipient_ed25519_pub)?;
    let wrapping_key = derive_wrapping_key(&shared);
    aes_gcm_encrypt(&wrapping_key, mek_wire_bytes)
}

/// Unwrap MEK wire bytes received from a sender.
pub fn unwrap_mek(
    recipient_signing_key: &SigningKey,
    sender_ed25519_pub: &[u8; 32],
    wrapped: &[u8],
) -> Result<Vec<u8>> {
    let shared = ecdh_shared_secret(recipient_signing_key, sender_ed25519_pub)?;
    let wrapping_key = derive_wrapping_key(&shared);
    aes_gcm_decrypt(&wrapping_key, wrapped)
}

fn ecdh_shared_secret(
    local_signing: &SigningKey,
    remote_ed25519_pub: &[u8; 32],
) -> Result<x25519_dalek::SharedSecret> {
    // Convert Ed25519 signing key to X25519 static secret using the
    // SHA-512-expanded scalar (same convention as ed25519-dalek's
    // ExpandedSecretKey). This matches the Montgomery conversion of
    // the public key so ECDH produces the correct shared secret.
    let local_x25519 = x25519_dalek::StaticSecret::from(local_signing.to_scalar_bytes());

    let remote_verifying = VerifyingKey::from_bytes(remote_ed25519_pub).map_err(|e| {
        TransportError::MekUnwrapFailed {
            reason: format!("invalid key: {e}"),
        }
    })?;
    let remote_x25519 = X25519PublicKey::from(remote_verifying.to_montgomery().to_bytes());
    Ok(local_x25519.diffie_hellman(&remote_x25519))
}

fn derive_wrapping_key(shared: &x25519_dalek::SharedSecret) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(None, shared.as_bytes());
    let mut key = [0u8; 32];
    hkdf.expand(MEK_WRAP_HKDF_INFO, &mut key)
        .expect("32-byte output valid for HKDF-SHA256");
    key
}

fn aes_gcm_encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| TransportError::EncryptionFailed {
        reason: e.to_string(),
    })?;
    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| TransportError::EncryptionFailed {
            reason: e.to_string(),
        })?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

fn aes_gcm_decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 12 {
        return Err(TransportError::MekUnwrapFailed {
            reason: "too short".into(),
        });
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| TransportError::MekUnwrapFailed {
        reason: e.to_string(),
    })?;
    let nonce = Nonce::from_slice(&data[..12]);
    cipher
        .decrypt(nonce, &data[12..])
        .map_err(|e| TransportError::MekUnwrapFailed {
            reason: e.to_string(),
        })
}

// ── Unified MEK cache ────────────────────────────────────────────────

/// A cached MEK entry with metadata for introspection.
struct CachedMek {
    mek: Mek,
    cached_at: Instant,
}

/// Unified MEK cache. Single source of truth, replaces the old dual cache.
///
/// Key: `(community_id, channel_id)`. For community-wide MEK, use empty
/// string as channel_id.
pub struct MekCache {
    entries: HashMap<(String, String), Vec<CachedMek>>,
}

impl MekCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Store a MEK at a specific generation. Deduplicates by generation.
    pub fn insert(&mut self, community_id: &str, channel_id: &str, mek: Mek) {
        let key = (community_id.to_string(), channel_id.to_string());
        let generations = self.entries.entry(key).or_default();
        let gen = mek.generation;
        if !generations.iter().any(|cm| cm.mek.generation == gen) {
            generations.push(CachedMek {
                mek,
                cached_at: Instant::now(),
            });
            generations.sort_by_key(|cm| cm.mek.generation);
        }
    }

    /// Get the current (latest generation) MEK for a channel.
    pub fn current(&self, community_id: &str, channel_id: &str) -> Option<&Mek> {
        self.entries
            .get(&(community_id.to_string(), channel_id.to_string()))
            .and_then(|gens| gens.last())
            .map(|cm| &cm.mek)
    }

    /// Get a specific generation MEK for a channel.
    pub fn get_generation(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<&Mek> {
        self.entries
            .get(&(community_id.to_string(), channel_id.to_string()))
            .and_then(|gens| gens.iter().find(|cm| cm.mek.generation == generation))
            .map(|cm| &cm.mek)
    }

    /// Remove all MEKs for a community.
    pub fn remove_community(&mut self, community_id: &str) {
        self.entries.retain(|(cid, _), _| cid != community_id);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Point-in-time snapshot of cached MEKs for a community.
    ///
    /// Returns display-ready data suitable for `rekindle key mek list`
    /// and the TUI dashboard. Sorted by channel then generation.
    pub fn snapshot(&self, community_id: &str) -> Vec<MekCacheEntrySnapshot> {
        let mut result: Vec<MekCacheEntrySnapshot> = self
            .entries
            .iter()
            .filter(|((cid, _), _)| cid == community_id)
            .flat_map(|((_, channel_id), entries)| {
                entries.iter().map(move |cm| MekCacheEntrySnapshot {
                    channel_id: channel_id.clone(),
                    generation: cm.mek.generation,
                    age_secs: cm.cached_at.elapsed().as_secs(),
                })
            })
            .collect();
        result.sort_by(|a, b| {
            a.channel_id
                .cmp(&b.channel_id)
                .then(a.generation.cmp(&b.generation))
        });
        result
    }

    /// Total number of cached MEK entries across all communities.
    pub fn total_entries(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    /// Number of unique (community, channel) pairs with cached MEKs.
    pub fn channel_count(&self) -> usize {
        self.entries.len()
    }
}

impl Default for MekCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Re-exported from `rekindle_types::display` — the SSOT definition.
pub use rekindle_types::display::MekCacheEntrySnapshot;
