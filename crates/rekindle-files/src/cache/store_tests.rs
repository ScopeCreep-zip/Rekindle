use super::*;

#[test]
fn locked_store_refuses_and_close_forgets() {
    let store = FileCacheStore::default();
    assert!(matches!(
        store.ensure_open("c"),
        Err(FilesError::IdentityNotLoaded)
    ));
    let dir = tempfile::tempdir().expect("tempdir");
    store.open(dir.path().to_path_buf(), Zeroizing::new([7u8; 32]));
    store.ensure_open("c").expect("opens");
    let id = Uuid::from_bytes([1; 16]);
    store
        .with_cache_mut("c", &mut |cache, pinned| {
            cache.insert(id, 0, b"chunk", pinned)
        })
        .expect("insert");
    assert_eq!(store.holder_of(id).as_deref(), Some("c"));
    store.close();
    assert_eq!(store.holder_of(id), None);
    assert!(store.with_cache("c", &mut |_| Ok(())).is_err());
}
