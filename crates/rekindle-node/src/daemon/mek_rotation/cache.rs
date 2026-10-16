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

use parking_lot::RwLock;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::ChannelMekCache;
use rekindle_transport::crypto::mek::MekCache;
use rekindle_types::channel_keys::KeyScope;

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
        scope: KeyScope,
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
            .get_generation(community_id, scope, generation)
            .cloned()
    }

    fn current(&self, community_id: &str, scope: KeyScope) -> Option<MediaEncryptionKey> {
        self.inner.read().current(community_id, scope).cloned()
    }

    /// Accept a key only if it is genuinely newer, or wins the
    /// same-generation tiebreak.
    ///
    /// Three cases, matching the desktop's `install_channel_mek`: a lower
    /// generation is a downgrade and is refused; a higher one replaces;
    /// an equal one is resolved by election rank so every peer converges
    /// on the same bytes regardless of arrival order.
    fn insert(&self, community_id: &str, scope: KeyScope, mek: MediaEncryptionKey) -> bool {
        let incoming_generation = mek.generation();

        let cached = self
            .inner
            .read()
            .current(community_id, scope)
            .map(|c| (c.generation(), c.election_rank()));

        if let Some((cached_generation, cached_rank)) = cached {
            if incoming_generation < cached_generation {
                tracing::debug!(
                    community = %community_id,
                    %scope,
                    cached = cached_generation,
                    incoming = incoming_generation,
                    "refusing MEK downgrade"
                );
                return false;
            }
            if incoming_generation == cached_generation
                && !rekindle_mek_rotation::convergence::incoming_wins_same_generation(
                    cached_rank.as_ref(),
                    mek.election_rank().as_ref(),
                )
            {
                return false;
            }
        }

        // `replace_generation`, not `insert`: `insert` dedupes by
        // generation and would silently drop a same-generation key that
        // has just *won* the tiebreak above.
        self.inner
            .write()
            .replace_generation(community_id, scope, mek);
        true
    }
}

/// The daemon's key history is its `MekCache`, which retains every
/// generation it has held for the process lifetime. Persisted history
/// (surviving a restart) arrives with the shared per-identity store
/// (plan D-phase, step-19 item N1).
impl rekindle_mek_rotation::MekHistory for MekCacheAdapter {
    fn load(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        ChannelMekCache::get(self, community_id, scope, generation)
    }
}

/// Reload the vault's latest MEK of every scope the unlocked session knows
/// (each community, and each of its channels with a record) into the
/// cache, through the convergence rule (plan D1: the keys outlive a
/// restart).
pub fn restore_from_vault(ctx: &crate::daemon::dispatch::DaemonContext) {
    let scopes: Vec<(String, KeyScope)> = ctx
        .session
        .read()
        .as_ref()
        .map(|session| {
            session
                .communities
                .values()
                .flat_map(|m| {
                    std::iter::once((m.governance_key.clone(), KeyScope::Community)).chain(
                        m.channel_record_keys.keys().filter_map(|channel| {
                            rekindle_types::id::ChannelId::from_hex(channel)
                                .map(|id| (m.governance_key.clone(), KeyScope::Channel(id)))
                        }),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let guard = ctx.vault.lock();
    let Some(vault) = guard.as_ref() else {
        return;
    };
    let cache = MekCacheAdapter::new(Arc::clone(&ctx.mek_cache));
    let mut restored = 0usize;
    for (community, scope) in scopes {
        if let Some(mek) = rekindle_vault::typed::mek::load_latest_mek(vault, &community, scope) {
            if cache.insert(&community, scope, mek) {
                restored += 1;
            }
        }
    }
    tracing::info!(restored, "MEKs reloaded from the vault");
}

/// The key provider every daemon key consumer reads through (plan D6).
pub fn key_provider(
    ctx: &crate::daemon::dispatch::DaemonContext,
) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
    let cache = Arc::new(MekCacheAdapter::new(Arc::clone(&ctx.mek_cache)));
    Arc::new(rekindle_mek_rotation::MekKeyProvider::new(
        cache.clone(),
        cache,
    ))
}

#[cfg(test)]
mod tests {
    use super::MekCacheAdapter;
    use parking_lot::RwLock;
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_mek_rotation::ChannelMekCache;
    use rekindle_transport::crypto::mek::MekCache;
    use rekindle_types::channel_keys::KeyScope;
    use rekindle_types::id::ChannelId;
    use std::sync::Arc;

    const CH: KeyScope = KeyScope::Channel(ChannelId([0x33; 16]));

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
        a.get("c", CH, generation).map(|m| *m.as_bytes())
    }

    #[test]
    fn newer_generation_replaces() {
        let a = adapter();
        a.insert("c", CH, key(1, 1));
        a.insert("c", CH, key(2, 2));
        assert_eq!(a.current_generation("c", CH), 2);
        assert_eq!(cached_bytes(&a, 2), Some([2u8; 32]));
    }

    /// A stale delivery must not roll the community back onto a key the
    /// departed member still holds.
    #[test]
    fn older_generation_is_refused() {
        let a = adapter();
        a.insert("c", CH, key(2, 5));
        a.insert("c", CH, key(9, 4));
        assert_eq!(a.current_generation("c", CH), 5);
        assert_eq!(cached_bytes(&a, 5), Some([2u8; 32]));
    }

    /// The split-brain case. Two peers mint different bytes at the same
    /// generation; every peer must land on the lower-ranked one whatever
    /// order they arrive in, or the community splits into two halves that
    /// cannot read each other.
    #[test]
    fn same_generation_converges_on_lowest_rank_either_order() {
        let high_first = adapter();
        high_first.insert("c", CH, ranked(0xEE, 7, HIGH_RANK));
        high_first.insert("c", CH, ranked(0x11, 7, LOW_RANK));

        let low_first = adapter();
        low_first.insert("c", CH, ranked(0x11, 7, LOW_RANK));
        low_first.insert("c", CH, ranked(0xEE, 7, HIGH_RANK));

        assert_eq!(cached_bytes(&high_first, 7), cached_bytes(&low_first, 7));
        assert_eq!(cached_bytes(&low_first, 7), Some([0x11; 32]));
    }

    /// Re-delivery of the identical key must not thrash the cache.
    #[test]
    fn same_generation_same_rank_keeps_cached() {
        let a = adapter();
        a.insert("c", CH, ranked(1, 3, LOW_RANK));
        a.insert("c", CH, ranked(2, 3, LOW_RANK));
        assert_eq!(cached_bytes(&a, 3), Some([1u8; 32]));
    }

    /// A tagged key beats an untagged one at the same generation: the
    /// untagged one came from a peer that cannot participate in the
    /// tiebreak, so deferring to it would leave the rest diverged.
    #[test]
    fn tagged_beats_untagged_at_equal_generation() {
        let a = adapter();
        a.insert("c", CH, key(1, 4));
        a.insert("c", CH, ranked(2, 4, HIGH_RANK));
        assert_eq!(cached_bytes(&a, 4), Some([2u8; 32]));
    }
}
