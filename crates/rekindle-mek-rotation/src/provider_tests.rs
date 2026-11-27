use super::*;
use crate::cache::InMemoryMekCache;
use std::collections::HashMap;

const VOICE: ChannelId = ChannelId([2; 16]);

#[derive(Default)]
struct History(HashMap<(KeyScope, u64), MediaEncryptionKey>);

impl MekHistory for History {
    fn load(&self, _: &str, scope: KeyScope, generation: u64) -> Option<MediaEncryptionKey> {
        self.0.get(&(scope, generation)).cloned()
    }
}

fn mek(byte: u8, generation: u64) -> MediaEncryptionKey {
    MediaEncryptionKey::from_bytes([byte; 32], generation)
}

fn provider(history: History) -> (MekKeyProvider, InMemoryMekCache) {
    let cache = InMemoryMekCache::new();
    let provider = MekKeyProvider::new(Arc::new(cache.clone()), Arc::new(history));
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
}
