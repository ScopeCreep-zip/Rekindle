//! The MEK implementation of the channel-key seam (plan D6, step B5).
//!
//! [`MekKeyProvider`] answers [`ChannelKeyProvider`] from a host's live
//! [`ChannelMekCache`] and, for generations no longer live, its persisted
//! [`MekHistory`]. A key is returned only for the exact `(community, scope,
//! generation)` asked for — the RFC 9605 §4.4.1 rule that a ciphertext's
//! key is selected by the id it names — and receivers keep superseded
//! generations so history stays readable (Megolm: inbound sessions keep
//! their earliest ratchet value).
//!
//! The community text key is one AES-256-GCM key with random 96-bit
//! nonces shared by every member, so NIST SP 800-38D §8.3 bounds it to
//! 2^32 encryptions across all senders. Departure rotation replaces it far
//! below that.

use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::channel_keys::{ChannelKeyProvider, KeyEpoch, KeyScope};
use rekindle_types::id::ChannelId;
use zeroize::Zeroizing;

use crate::deps::ChannelMekCache;

/// Persisted key generations: what a host stored when each key arrived.
pub trait MekHistory: Send + Sync {
    /// The stored key for exactly `(community_id, scope, generation)`.
    fn load(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Option<MediaEncryptionKey>;
}

/// [`ChannelKeyProvider`] over a host's MEK cache and history.
pub struct MekKeyProvider {
    cache: Arc<dyn ChannelMekCache>,
    history: Arc<dyn MekHistory>,
}

impl MekKeyProvider {
    #[must_use]
    pub fn new(cache: Arc<dyn ChannelMekCache>, history: Arc<dyn MekHistory>) -> Self {
        Self { cache, history }
    }
}

impl ChannelKeyProvider for MekKeyProvider {
    fn current_epoch(&self, community_id: &str, scope: KeyScope) -> Option<KeyEpoch> {
        self.cache
            .current(community_id, scope)
            .map(|mek| KeyEpoch(mek.generation()))
    }

    fn key(
        &self,
        community_id: &str,
        scope: KeyScope,
        epoch: KeyEpoch,
    ) -> Option<Zeroizing<[u8; 32]>> {
        let mek = self
            .cache
            .get(community_id, scope, epoch.0)
            .or_else(|| self.history.load(community_id, scope, epoch.0))?;
        // The stored generation is part of the key's identity: a history
        // entry that disagrees with the slot it was read from is not the
        // key for this epoch.
        (mek.generation() == epoch.0).then(|| Zeroizing::new(*mek.as_bytes()))
    }

    fn scope_for_text(&self, _community_id: &str, _channel: ChannelId) -> KeyScope {
        // Plan D6: channel text, its attachments and threads use the
        // community key. Per-channel text keys arrive with private
        // channels (D11, step E3.5).
        KeyScope::Community
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
