use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_transport::crypto::mek::MekCache;
use rekindle_types::channel_keys::{KeyEpoch, KeyScope};
use rekindle_vault::typed::mek::persist_mek;
use rekindle_vault::VaultStore;

/// After an unlock the cache holds only each scope's latest generation
/// (`restore_stored_meks`); an older generation's posts still open,
/// from the vault.
#[test]
fn a_generation_the_cache_lacks_is_read_from_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, _) = VaultStore::create(&dir.path().join("pk.vault"), "pass").unwrap();
    for generation in [1, 2] {
        let mek =
            MediaEncryptionKey::from_bytes([u8::try_from(generation).unwrap(); 32], generation);
        persist_mek(&vault, "c", KeyScope::Community, &mek).unwrap();
    }
    let handle = rekindle_vault::new_handle();
    *handle.lock() = Some(vault);
    let cache = Arc::new(RwLock::new(MekCache::new()));
    cache.write().replace_generation(
        "c",
        KeyScope::Community,
        MediaEncryptionKey::from_bytes([2; 32], 2),
    );
    let keys = crate::daemon::mek_rotation::provider_over(cache, handle.clone());

    let key = |generation| keys.key("c", KeyScope::Community, KeyEpoch(generation));
    assert_eq!(key(2).map(|k| *k), Some([2; 32]), "the cached generation");
    assert_eq!(
        key(1).map(|k| *k),
        Some([1; 32]),
        "an older one, from the vault"
    );
    assert!(key(3).is_none(), "never a generation nobody stored");
    *handle.lock() = None;
    assert!(key(1).is_none(), "nothing from a locked vault");
}
