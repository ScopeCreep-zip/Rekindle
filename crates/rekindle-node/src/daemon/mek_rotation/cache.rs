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

/// The key provider every daemon key consumer reads through (plan D6).
pub fn key_provider(
    ctx: &crate::daemon::dispatch::DaemonContext,
) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
    provider_over(Arc::clone(&ctx.mek_cache), ctx.vault.clone())
}

/// The provider over `mek_cache` and, for a generation it does not hold,
/// `vault`: the cache keeps what this process installed, and an unlock
/// restores only each scope's latest key, so older generations come from
/// the vault, the store of record (A/B R3-9).
#[must_use]
pub fn provider_over(
    mek_cache: Arc<RwLock<MekCache>>,
    vault: rekindle_vault::VaultHandle,
) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
    Arc::new(rekindle_mek_rotation::MekKeyProvider::new(
        Arc::new(MekCacheAdapter::new(mek_cache)),
        Arc::new(crate::host::adapters::mek::VaultMekPersist::new(vault)),
    ))
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
