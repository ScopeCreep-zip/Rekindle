//! Cache and persistence backings for the daemon's MEK rotation.
//!
//! `rekindle-mek-rotation` and `rekindle_transport::crypto::mek::MekCache`
//! now speak the *same* type — `rekindle_secrets::keys::MediaEncryptionKey`,
//! re-exported by transport as `Mek` — so this is a passthrough rather
//! than a conversion.
//!
//! It did not start that way. Transport held a third implementation of
//! the 40-byte MEK format with no provenance field, so every insert
//! dropped the `election_rank` that
//! `convergence::incoming_wins_same_generation` needs to resolve a
//! same-generation split-brain, and the ranks had to ride in a side map
//! keyed alongside the cache. Making Tier 2's type canonical deleted the
//! workaround rather than the symptom: the key carries its own
//! provenance, so the comparison reads it straight off the cached value.
//!
//! `insert` still applies the three-way rule (refuse downgrade / accept
//! newer / tiebreak equal by rank), because that is protocol policy and
//! a cache should not silently arbitrate it — see
//! `MekCache::replace_generation`.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::{ChannelMekCache, MekPersist, MekRotationError};
use rekindle_transport::crypto::mek::{Mek, MekCache};

/// Presents the daemon's `MekCache` through the rotation crate's trait.
pub struct MekCacheAdapter {
    inner: Arc<RwLock<MekCache>>,
}

impl MekCacheAdapter {
    #[must_use]
    pub fn new(inner: Arc<RwLock<MekCache>>) -> Self {
        Self { inner }
    }
}

impl ChannelMekCache for MekCacheAdapter {
    fn get(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        // `get_generation` rather than `current` + filter: the cache
        // retains superseded generations, and a rotation that has
        // already advanced past `generation` must still be able to read
        // the older key it is replacing.
        //
        // A plain clone now — the cached value *is* a
        // `MediaEncryptionKey`, provenance included, so nothing has to be
        // reconstructed or re-attached on the way out.
        self.inner
            .read()
            .get_generation(community_id, channel_id, generation)
            .cloned()
    }

    /// Accept a key only if it is genuinely newer, or wins the
    /// same-generation tiebreak.
    ///
    /// Three cases, matching the desktop's `install_channel_mek`: a lower
    /// generation is a downgrade and is refused; a higher one replaces;
    /// an equal one is resolved by election rank so every peer converges
    /// on the same bytes regardless of arrival order.
    fn insert(&self, community_id: &str, channel_id: &str, mek: MediaEncryptionKey) {
        let incoming_generation = mek.generation();

        let cached = self
            .inner
            .read()
            .current(community_id, channel_id)
            .map(|c| (c.generation(), c.election_rank()));

        if let Some((cached_generation, cached_rank)) = cached {
            if incoming_generation < cached_generation {
                tracing::debug!(
                    community = %community_id,
                    channel = %channel_id,
                    cached = cached_generation,
                    incoming = incoming_generation,
                    "refusing MEK downgrade"
                );
                return;
            }
            if incoming_generation == cached_generation
                && !rekindle_mek_rotation::convergence::incoming_wins_same_generation(
                    cached_rank.as_ref(),
                    mek.election_rank().as_ref(),
                )
            {
                return;
            }
        }

        // `replace_generation`, not `insert`: `insert` dedupes by
        // generation and would silently drop a same-generation key that
        // has just *won* the tiebreak above.
        self.inner
            .write()
            .replace_generation(community_id, channel_id, mek);
    }

    fn current_generation(&self, community_id: &str, channel_id: &str) -> u64 {
        self.inner
            .read()
            .current(community_id, channel_id)
            .map_or(0, Mek::generation)
    }
}

/// Durable MEK storage for the daemon.
///
/// The desktop writes MEKs into Stronghold; the daemon has no
/// Stronghold, so this uses the OS keyring through the same
/// `state::keystore` helpers that already hold the signing key and the
/// registry keypairs. Labels are a hash of the
/// `(community, channel, generation)` triple so a long channel id
/// cannot produce an unbounded keyring entry name, and so the label
/// reveals nothing about which communities this daemon belongs to.
///
/// Note that `rekindle-mek-rotation` does not yet call `persist()` on
/// any path — the trait is declared and the desktop implements it, but
/// no orchestrator invokes it. This is a real implementation rather
/// than a stub so that wiring it up is the only remaining step.
pub struct DaemonMekPersist;

fn label_for(community_id: &str, channel_id: &str, generation: u64) -> String {
    let composite = format!("{community_id}\u{1f}{channel_id}\u{1f}{generation}");
    format!(
        "mek-{}",
        &rekindle_utils::blake3_hex(composite.as_bytes())[..32]
    )
}

#[async_trait]
impl MekPersist for DaemonMekPersist {
    async fn store_mek_for_generation(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
        wrapped_bytes: Vec<u8>,
    ) -> Result<(), MekRotationError> {
        crate::state::keystore::store_keypair_bytes(
            &label_for(community_id, channel_id, generation),
            &wrapped_bytes,
        )
        .await
        .map_err(|e| MekRotationError::Persist(format!("store MEK: {e}")))
    }

    async fn load_mek_for_generation(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Result<Option<Vec<u8>>, MekRotationError> {
        crate::state::keystore::load_keypair_bytes(&label_for(community_id, channel_id, generation))
            .await
            .map_err(|e| MekRotationError::Persist(format!("load MEK: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::{label_for, MekCacheAdapter};
    use parking_lot::RwLock;
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_mek_rotation::ChannelMekCache;
    use rekindle_transport::crypto::mek::MekCache;
    use std::sync::Arc;

    const LOW_RANK: [u8; 32] = [0x10; 32];
    const HIGH_RANK: [u8; 32] = [0x20; 32];

    fn adapter() -> MekCacheAdapter {
        MekCacheAdapter::new(Arc::new(RwLock::new(MekCache::new())))
    }

    fn key(byte: u8, generation: u64) -> MediaEncryptionKey {
        MediaEncryptionKey::from_bytes([byte; 32], generation)
    }

    fn ranked(byte: u8, generation: u64, rank: [u8; 32]) -> MediaEncryptionKey {
        key(byte, generation).with_provenance([0xaa; 32], rank)
    }

    fn cached_bytes(a: &MekCacheAdapter, generation: u64) -> Option<[u8; 32]> {
        a.get("c", "ch", generation).map(|m| *m.as_bytes())
    }

    #[test]
    fn newer_generation_replaces() {
        let a = adapter();
        a.insert("c", "ch", key(1, 1));
        a.insert("c", "ch", key(2, 2));
        assert_eq!(a.current_generation("c", "ch"), 2);
        assert_eq!(cached_bytes(&a, 2), Some([2u8; 32]));
    }

    /// A stale delivery must not roll the community back onto a key the
    /// departed member still holds.
    #[test]
    fn older_generation_is_refused() {
        let a = adapter();
        a.insert("c", "ch", key(2, 5));
        a.insert("c", "ch", key(9, 4));
        assert_eq!(a.current_generation("c", "ch"), 5);
        assert_eq!(cached_bytes(&a, 5), Some([2u8; 32]));
    }

    /// The split-brain case. Two peers mint different bytes at the same
    /// generation; every peer must land on the lower-ranked one whatever
    /// order they arrive in, or the community splits into two halves that
    /// cannot read each other.
    #[test]
    fn same_generation_converges_on_lowest_rank_either_order() {
        let high_first = adapter();
        high_first.insert("c", "ch", ranked(0xEE, 7, HIGH_RANK));
        high_first.insert("c", "ch", ranked(0x11, 7, LOW_RANK));

        let low_first = adapter();
        low_first.insert("c", "ch", ranked(0x11, 7, LOW_RANK));
        low_first.insert("c", "ch", ranked(0xEE, 7, HIGH_RANK));

        assert_eq!(cached_bytes(&high_first, 7), cached_bytes(&low_first, 7));
        assert_eq!(cached_bytes(&low_first, 7), Some([0x11; 32]));
    }

    /// Re-delivery of the identical key must not thrash the cache.
    #[test]
    fn same_generation_same_rank_keeps_cached() {
        let a = adapter();
        a.insert("c", "ch", ranked(1, 3, LOW_RANK));
        a.insert("c", "ch", ranked(2, 3, LOW_RANK));
        assert_eq!(cached_bytes(&a, 3), Some([1u8; 32]));
    }

    /// A tagged key beats an untagged one at the same generation: the
    /// untagged one came from a peer that cannot participate in the
    /// tiebreak, so deferring to it would leave the rest diverged.
    #[test]
    fn tagged_beats_untagged_at_equal_generation() {
        let a = adapter();
        a.insert("c", "ch", key(1, 4));
        a.insert("c", "ch", ranked(2, 4, HIGH_RANK));
        assert_eq!(cached_bytes(&a, 4), Some([2u8; 32]));
    }

    #[test]
    fn label_is_bounded_and_distinct_per_triple() {
        let a = label_for("gov", "chan", 1);
        let b = label_for("gov", "chan", 2);
        let c = label_for("gov", "other", 1);
        assert_ne!(a, b, "generation must change the label");
        assert_ne!(a, c, "channel must change the label");
        assert_eq!(a.len(), 36, "mek- prefix plus 32 hex chars");
    }

    /// The separator matters: without it `("ab", "c")` and `("a", "bc")`
    /// would hash identically and two different channels would share one
    /// keyring entry, silently overwriting each other's key.
    #[test]
    fn field_boundaries_are_unambiguous() {
        assert_ne!(label_for("ab", "c", 1), label_for("a", "bc", 1));
    }

    /// The label must not leak the governance key it belongs to — a
    /// keyring listing is readable by anything running as this user.
    #[test]
    fn label_does_not_embed_the_community_id() {
        let community = "0123456789abcdef0123456789abcdef";
        let label = label_for(community, "general", 3);
        assert!(!label.contains(community));
        assert!(!label.contains(&community[..8]));
    }
}
