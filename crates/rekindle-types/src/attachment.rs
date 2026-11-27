//! Lost Cargo file-attachment data types (Tier 1).
//!
//! Pure data — wire format for `AttachmentOffer` (embedded in
//! `ChannelEntry::Message`) and `AttachmentBitmap` (peer-possession
//! advertisement in `ChannelEntry::AttachmentCached`).
//!
//! The chunker, cache, and verify logic that *uses* these types lives in
//! `rekindle-files` (Tier 7). That crate re-exports `AttachmentOffer` and
//! `AttachmentBitmap` for ergonomics.
//!
//! See the architecture spec §28.9 (Lost Cargo) and the design departures
//! documented in the migration plan §1.J (per-file FEK, chunk bitmap, …).

use serde::{Deserialize, Serialize};

/// Embedded in `ChannelEntry::Message.attachment` per architecture §28.9
/// lines 3233-3244, with `wrapped_fek` / `fek_mek_generation` added for the
/// Signal/Matrix per-file-FEK pattern (plan §1.J1) so blob chunks survive
/// MEK rotation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentOffer {
    /// 16-byte UUIDv4 assigned by the uploader.
    pub attachment_id: [u8; 16],
    pub filename: String,
    pub mime_type: String,
    pub total_size: u64,
    pub chunk_count: u32,
    /// Always 28 KiB for v1; serialized so future versions can negotiate
    /// a different size without breaking older readers.
    pub chunk_size: u32,
    /// Flat-list Merkle root (v1) — `SHA256(chunk_hashes concatenated)`.
    /// v2 will switch to BEP-52 binary tree (plan §1.J6).
    pub merkle_root: [u8; 32],
    /// SHA-256 of plaintext chunk `i`. Length MUST equal `chunk_count`.
    pub chunk_hashes: Vec<[u8; 32]>,
    /// Per-file FEK encrypted under the channel MEK at upload time
    /// (plan §1.J1).
    pub wrapped_fek: Vec<u8>,
    /// MEK generation used to wrap `wrapped_fek`. Receivers use this to
    /// pick the right historical channel MEK on download.
    pub fek_mek_generation: u64,
}

/// Bitmap recording which chunks of an attachment a peer holds locally
/// (plan §1.J4). 1 bit per chunk; LSB-first within each byte.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentBitmap {
    bytes: Vec<u8>,
    chunk_count: u32,
}

impl AttachmentBitmap {
    pub fn new(chunk_count: u32) -> Self {
        Self {
            bytes: vec![0u8; chunk_count.div_ceil(8) as usize],
            chunk_count,
        }
    }

    pub fn full(chunk_count: u32) -> Self {
        let mut bm = Self::new(chunk_count);
        for i in 0..chunk_count {
            let byte = (i / 8) as usize;
            let bit = (i % 8) as u8;
            bm.bytes[byte] |= 1 << bit;
        }
        bm
    }

    pub fn chunk_count(&self) -> u32 {
        self.chunk_count
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Build a bitmap from raw bytes. Returns `None` if the byte length
    /// does not match `ceil(chunk_count / 8)`.
    pub fn from_bytes(bytes: Vec<u8>, chunk_count: u32) -> Option<Self> {
        let expected = chunk_count.div_ceil(8) as usize;
        if bytes.len() != expected {
            return None;
        }
        Some(Self { bytes, chunk_count })
    }

    /// Set the bit for `chunk_index`. Returns `false` (no-op) if the index
    /// is out of range.
    pub fn set(&mut self, chunk_index: u32) -> bool {
        if chunk_index >= self.chunk_count {
            return false;
        }
        let byte = (chunk_index / 8) as usize;
        let bit = (chunk_index % 8) as u8;
        self.bytes[byte] |= 1 << bit;
        true
    }

    /// Clear the bit for `chunk_index`. Returns `false` if out of range.
    pub fn clear(&mut self, chunk_index: u32) -> bool {
        if chunk_index >= self.chunk_count {
            return false;
        }
        let byte = (chunk_index / 8) as usize;
        let bit = (chunk_index % 8) as u8;
        self.bytes[byte] &= !(1 << bit);
        true
    }

    pub fn has(&self, chunk_index: u32) -> bool {
        if chunk_index >= self.chunk_count {
            return false;
        }
        let byte = (chunk_index / 8) as usize;
        let bit = (chunk_index % 8) as u8;
        self.bytes[byte] & (1 << bit) != 0
    }

    pub fn count(&self) -> u32 {
        self.bytes.iter().map(|b| b.count_ones()).sum()
    }

    pub fn is_complete(&self) -> bool {
        self.count() == self.chunk_count
    }

    /// Indices the peer is missing.
    pub fn missing(&self) -> Vec<u32> {
        (0..self.chunk_count).filter(|i| !self.has(*i)).collect()
    }

    /// Indices both peers have. Useful for download routing — pick chunks
    /// the requester is missing AND the responder holds.
    pub fn intersect(&self, other: &Self) -> Vec<u32> {
        let limit = self.chunk_count.min(other.chunk_count);
        (0..limit)
            .filter(|i| self.has(*i) && other.has(*i))
            .collect()
    }
}

#[cfg(test)]
#[path = "attachment_tests.rs"]
mod tests;
