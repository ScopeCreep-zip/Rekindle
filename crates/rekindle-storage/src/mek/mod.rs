//! MEK (Media Encryption Key) cache persistence.
//!
//! MEKs are per-channel AES-256-GCM keys at a generation number.
//! Loaded into memory on daemon unlock. Flushed to vault on lock.

pub mod cache;

/// A single MEK entry loaded from or stored to the vault.
#[derive(Debug, Clone)]
pub struct MekEntry {
    pub community_id: String,
    pub channel_id: String,
    pub generation: u64,
    pub key_bytes: [u8; 32],
}
