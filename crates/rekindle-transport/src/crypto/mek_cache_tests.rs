use super::{Mek, MekCache};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::ChannelId;

const CH: KeyScope = KeyScope::Channel(ChannelId([7; 16]));

/// `insert` is idempotent by generation — the property every
/// re-delivery path relies on.
#[test]
fn insert_keeps_the_first_key_at_a_generation() {
    let mut cache = MekCache::new();
    cache.insert("c", CH, Mek::from_bytes([1; 32], 3));
    cache.insert("c", CH, Mek::from_bytes([2; 32], 3));
    assert_eq!(
        cache.get_generation("c", CH, 3).unwrap().as_bytes(),
        &[1; 32]
    );
}

/// …which is exactly why `replace_generation` exists. Without it a
/// same-generation split-brain resolves to "whoever arrived first",
/// which is a different answer on every peer.
#[test]
fn replace_generation_overwrites_at_the_same_generation() {
    let mut cache = MekCache::new();
    cache.insert("c", CH, Mek::from_bytes([1; 32], 3));
    cache.replace_generation("c", CH, Mek::from_bytes([2; 32], 3));
    assert_eq!(
        cache.get_generation("c", CH, 3).unwrap().as_bytes(),
        &[2; 32]
    );
}

/// It must also insert when the generation is absent, so callers do
/// not have to branch.
#[test]
fn replace_generation_inserts_when_absent() {
    let mut cache = MekCache::new();
    cache.replace_generation("c", CH, Mek::from_bytes([7; 32], 9));
    assert_eq!(cache.current("c", CH).unwrap().generation(), 9);
}

/// Replacing must not disturb the retained older generations that
/// `get_generation` serves for late-arriving ciphertext.
#[test]
fn replace_generation_leaves_other_generations_intact() {
    let mut cache = MekCache::new();
    cache.insert("c", CH, Mek::from_bytes([1; 32], 1));
    cache.insert("c", CH, Mek::from_bytes([2; 32], 2));
    cache.replace_generation("c", CH, Mek::from_bytes([9; 32], 2));
    assert_eq!(
        cache.get_generation("c", CH, 1).unwrap().as_bytes(),
        &[1; 32]
    );
    assert_eq!(
        cache.get_generation("c", CH, 2).unwrap().as_bytes(),
        &[9; 32]
    );
    assert_eq!(cache.current("c", CH).unwrap().generation(), 2);
}

/// The community key and a channel key never share an entry.
#[test]
fn community_and_channel_scopes_are_distinct() {
    let mut cache = MekCache::new();
    cache.insert("c", KeyScope::Community, Mek::from_bytes([1; 32], 4));
    cache.insert("c", CH, Mek::from_bytes([2; 32], 1));
    assert_eq!(
        cache
            .current("c", KeyScope::Community)
            .unwrap()
            .generation(),
        4
    );
    assert_eq!(cache.current("c", CH).unwrap().generation(), 1);
    assert_eq!(cache.channels("c"), vec![ChannelId([7; 16])]);
    let snapshot = cache.snapshot("c");
    assert_eq!(snapshot[0].channel_id, None);
    assert_eq!(snapshot[1].channel_id, Some(hex::encode([7u8; 16])));
}
