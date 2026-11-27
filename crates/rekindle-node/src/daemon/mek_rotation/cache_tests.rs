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
