//! Cache and persistence backings for the daemon's MEK rotation.
//!
//! `rekindle-mek-rotation` speaks `MediaEncryptionKey`; the daemon's
//! store is `rekindle_transport::crypto::mek::MekCache`, holding `Mek`.
//! Both are a 32-byte key plus a generation, so the adapter converts
//! rather than introducing a second cache — two stores would drift
//! exactly when it matters, mid-rotation.

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
        self.inner
            .read()
            .get_generation(community_id, channel_id, generation)
            .map(|mek| MediaEncryptionKey::from_bytes(*mek.as_bytes(), mek.generation()))
    }

    fn insert(&self, community_id: &str, channel_id: &str, mek: MediaEncryptionKey) {
        self.inner.write().insert(
            community_id,
            channel_id,
            Mek::from_bytes(*mek.as_bytes(), mek.generation()),
        );
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
    use super::label_for;

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
