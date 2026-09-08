//! Unified MEK (Media Encryption Key) resolution and lifecycle.
//!
//! Single cache replacing the dual `mek_cache` + `channel_mek_cache`.
//! Handles generation lookup, and delegates the wrap/unwrap algorithm
//! (ECDH + HKDF + AES-GCM) to `rekindle_crypto::group::mek_distribution`
//! — the single implementation shared by both tracks — and Stronghold
//! persistence to `rekindle-secrets`.

use std::collections::HashMap;
use std::time::Instant;

use ed25519_dalek::SigningKey;

use crate::error::{Result, TransportError};

/// The Media Encryption Key, re-exported from Tier 2.
///
/// This was a third implementation of the same 40-byte wire format
/// (`[generation LE(8) || key(32)]`) alongside
/// `rekindle_secrets::keys::MediaEncryptionKey` and
/// `rekindle_crypto::group::media_key::MediaEncryptionKey` — with the
/// same AES-GCM body and the same `[nonce(12) || ct+tag]` layout,
/// differing only in which error type it returned.
///
/// Being the *base* form was the real cost. Converting into it dropped
/// the 65-byte provenance suffix, so the daemon's `MekCacheAdapter` had
/// to carry election ranks in a side map to run
/// `convergence::incoming_wins_same_generation` — without which two
/// peers minting different bytes at one generation never converge. The
/// canonical type carries its own provenance and the workaround goes
/// with it.
///
/// `encrypt`/`decrypt` now return `CryptoError`; callers in this crate
/// convert through the existing `From<CryptoError> for TransportError`,
/// which maps encryption and decryption failures to their own variants
/// rather than stringifying.
pub use rekindle_secrets::keys::MediaEncryptionKey as Mek;

// ── MEK wrapping (ECDH + HKDF + AES-GCM) ────────────────────────────
//
// Thin wrappers over `rekindle_crypto::group::mek_distribution` — the
// single wrap/unwrap implementation shared by both tracks (same HKDF
// label `rekindle-mek-wrap-v1`, same `[12-byte nonce || ct+tag]` wire
// format). The transport-local copy of the algorithm was a
// wire-compatible fork kept in sync by hand; only the
// `CryptoError → TransportError` mapping lives here now.

/// Wrap MEK wire bytes for a specific recipient via X25519 ECDH.
///
/// Output: `[12-byte nonce || ciphertext+tag]` (68 bytes for 40-byte input).
pub fn wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_pub: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>> {
    rekindle_crypto::group::mek_distribution::wrap_mek(
        sender_signing_key,
        recipient_ed25519_pub,
        mek_wire_bytes,
    )
    .map_err(|e| match e {
        // Encryption failure keeps its own transport variant; everything
        // else collapses to the wrap/unwrap failure. The primitive
        // failures now live in Tier 1 behind `Core`, so the pattern
        // nests one level rather than naming a variant this crate owns.
        rekindle_crypto::error::CryptoError::Core(
            rekindle_crypto::error::CoreCryptoError::Encryption(reason),
        ) => TransportError::EncryptionFailed { reason },
        other => TransportError::MekUnwrapFailed {
            reason: other.to_string(),
        },
    })
}

/// Unwrap MEK wire bytes received from a sender.
pub fn unwrap_mek(
    recipient_signing_key: &SigningKey,
    sender_ed25519_pub: &[u8; 32],
    wrapped: &[u8],
) -> Result<Vec<u8>> {
    rekindle_crypto::group::mek_distribution::unwrap_mek(
        recipient_signing_key,
        sender_ed25519_pub,
        wrapped,
    )
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
        let gen = mek.generation();
        if !generations.iter().any(|cm| cm.mek.generation() == gen) {
            generations.push(CachedMek {
                mek,
                cached_at: Instant::now(),
            });
            generations.sort_by_key(|cm| cm.mek.generation());
        }
    }

    /// Overwrite the key held at `mek`'s generation, inserting if absent.
    ///
    /// [`Self::insert`] deduplicates by generation and therefore drops a
    /// *different* key arriving at a generation already held. That is the
    /// right default — it makes re-delivery idempotent — but it makes the
    /// same-generation split-brain unresolvable: two peers can mint
    /// different bytes at one generation, and whichever arrived first
    /// would stick, differently on every peer.
    ///
    /// Callers use this only after deciding the incoming key wins, via
    /// `rekindle_mek_rotation::convergence::incoming_wins_same_generation`.
    /// The decision stays out of this type on purpose — a cache should
    /// not arbitrate protocol conflicts — but the inputs are no longer
    /// out of reach: `Mek` is now the canonical key and carries its own
    /// `election_rank`, so a caller can compare without a side table.
    pub fn replace_generation(&mut self, community_id: &str, channel_id: &str, mek: Mek) {
        let key = (community_id.to_string(), channel_id.to_string());
        let generations = self.entries.entry(key).or_default();
        let gen = mek.generation();
        if let Some(existing) = generations.iter_mut().find(|cm| cm.mek.generation() == gen) {
            existing.mek = mek;
            existing.cached_at = Instant::now();
            return;
        }
        generations.push(CachedMek {
            mek,
            cached_at: Instant::now(),
        });
        generations.sort_by_key(|cm| cm.mek.generation());
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
            .and_then(|gens| gens.iter().find(|cm| cm.mek.generation() == generation))
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
                    generation: cm.mek.generation(),
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

#[cfg(test)]
mod cache_tests {
    use super::{Mek, MekCache};

    /// `insert` is idempotent by generation — the property every
    /// re-delivery path relies on.
    #[test]
    fn insert_keeps_the_first_key_at_a_generation() {
        let mut cache = MekCache::new();
        cache.insert("c", "ch", Mek::from_bytes([1; 32], 3));
        cache.insert("c", "ch", Mek::from_bytes([2; 32], 3));
        assert_eq!(
            cache.get_generation("c", "ch", 3).unwrap().as_bytes(),
            &[1; 32]
        );
    }

    /// …which is exactly why `replace_generation` exists. Without it a
    /// same-generation split-brain resolves to "whoever arrived first",
    /// which is a different answer on every peer.
    #[test]
    fn replace_generation_overwrites_at_the_same_generation() {
        let mut cache = MekCache::new();
        cache.insert("c", "ch", Mek::from_bytes([1; 32], 3));
        cache.replace_generation("c", "ch", Mek::from_bytes([2; 32], 3));
        assert_eq!(
            cache.get_generation("c", "ch", 3).unwrap().as_bytes(),
            &[2; 32]
        );
    }

    /// It must also insert when the generation is absent, so callers do
    /// not have to branch.
    #[test]
    fn replace_generation_inserts_when_absent() {
        let mut cache = MekCache::new();
        cache.replace_generation("c", "ch", Mek::from_bytes([7; 32], 9));
        assert_eq!(cache.current("c", "ch").unwrap().generation(), 9);
    }

    /// Replacing must not disturb the retained older generations that
    /// `get_generation` serves for late-arriving ciphertext.
    #[test]
    fn replace_generation_leaves_other_generations_intact() {
        let mut cache = MekCache::new();
        cache.insert("c", "ch", Mek::from_bytes([1; 32], 1));
        cache.insert("c", "ch", Mek::from_bytes([2; 32], 2));
        cache.replace_generation("c", "ch", Mek::from_bytes([9; 32], 2));
        assert_eq!(
            cache.get_generation("c", "ch", 1).unwrap().as_bytes(),
            &[1; 32]
        );
        assert_eq!(
            cache.get_generation("c", "ch", 2).unwrap().as_bytes(),
            &[9; 32]
        );
        assert_eq!(cache.current("c", "ch").unwrap().generation(), 2);
    }
}
