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
use std::time::Duration;

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

/// The governance fact media scoping depends on.
pub trait ChannelKinds: Send + Sync {
    /// Whether `channel` is a stage channel. Stage channels never rotate
    /// (architecture §10.7: anyone may listen), so their media is under
    /// the community key; every other channel's media is under its own
    /// key, rotated on each join and leave (§10.5).
    fn is_stage(&self, community_id: &str, channel: ChannelId) -> bool;
}

/// [`ChannelKeyProvider`] over a host's MEK cache and history.
pub struct MekKeyProvider {
    cache: Arc<dyn ChannelMekCache>,
    history: Arc<dyn MekHistory>,
    kinds: Arc<dyn ChannelKinds>,
}

impl MekKeyProvider {
    #[must_use]
    pub fn new(
        cache: Arc<dyn ChannelMekCache>,
        history: Arc<dyn MekHistory>,
        kinds: Arc<dyn ChannelKinds>,
    ) -> Self {
        Self {
            cache,
            history,
            kinds,
        }
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

    fn current_epoch_age(&self, community_id: &str, scope: KeyScope) -> Option<Duration> {
        self.cache.current_age(community_id, scope)
    }

    fn scope_for_text(&self, _community_id: &str, _channel: ChannelId) -> KeyScope {
        // Plan D6: channel text, its attachments and threads use the
        // community key. Per-channel text keys arrive with private
        // channels (D11, step E3.5).
        KeyScope::Community
    }

    fn scope_for_media(&self, community_id: &str, channel: ChannelId) -> KeyScope {
        if self.kinds.is_stage(community_id, channel) {
            KeyScope::Community
        } else {
            KeyScope::Channel(channel)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InMemoryMekCache;
    use std::collections::HashMap;

    const STAGE: ChannelId = ChannelId([1; 16]);
    const VOICE: ChannelId = ChannelId([2; 16]);

    #[derive(Default)]
    struct History(HashMap<(KeyScope, u64), MediaEncryptionKey>);

    impl MekHistory for History {
        fn load(&self, _: &str, scope: KeyScope, generation: u64) -> Option<MediaEncryptionKey> {
            self.0.get(&(scope, generation)).cloned()
        }
    }

    struct Kinds;

    impl ChannelKinds for Kinds {
        fn is_stage(&self, _: &str, channel: ChannelId) -> bool {
            channel == STAGE
        }
    }

    fn mek(byte: u8, generation: u64) -> MediaEncryptionKey {
        MediaEncryptionKey::from_bytes([byte; 32], generation)
    }

    fn provider(history: History) -> (MekKeyProvider, InMemoryMekCache) {
        let cache = InMemoryMekCache::new();
        let provider =
            MekKeyProvider::new(Arc::new(cache.clone()), Arc::new(history), Arc::new(Kinds));
        (provider, cache)
    }

    #[test]
    fn answers_current_and_historical_generations_exactly() {
        let mut history = History::default();
        history.0.insert((KeyScope::Community, 3), mek(3, 3));
        let (provider, cache) = provider(history);
        cache.insert("c", KeyScope::Community, mek(4, 4));

        assert_eq!(
            provider.current_epoch("c", KeyScope::Community),
            Some(KeyEpoch(4))
        );
        assert_eq!(
            *provider.key("c", KeyScope::Community, KeyEpoch(4)).unwrap(),
            [4; 32]
        );
        assert_eq!(
            *provider.key("c", KeyScope::Community, KeyEpoch(3)).unwrap(),
            [3; 32]
        );
        assert!(provider
            .key("c", KeyScope::Community, KeyEpoch(2))
            .is_none());
    }

    /// A channel's key is never answered with the community key, or the
    /// reverse — the substitution the old fallback chains made.
    #[test]
    fn never_substitutes_another_scope() {
        let (provider, cache) = provider(History::default());
        cache.insert("c", KeyScope::Community, mek(9, 1));
        let voice = KeyScope::Channel(VOICE);
        assert_eq!(provider.current_epoch("c", voice), None);
        assert!(provider.key("c", voice, KeyEpoch(1)).is_none());
    }

    #[test]
    fn history_entry_under_the_wrong_generation_is_refused() {
        let mut history = History::default();
        history.0.insert((KeyScope::Community, 2), mek(7, 5));
        let (provider, _) = provider(history);
        assert!(provider
            .key("c", KeyScope::Community, KeyEpoch(2))
            .is_none());
    }

    #[test]
    fn scope_policy() {
        let (provider, _) = provider(History::default());
        assert_eq!(provider.scope_for_text("c", VOICE), KeyScope::Community);
        assert_eq!(provider.scope_for_media("c", STAGE), KeyScope::Community);
        assert_eq!(
            provider.scope_for_media("c", VOICE),
            KeyScope::Channel(VOICE)
        );
    }
}
