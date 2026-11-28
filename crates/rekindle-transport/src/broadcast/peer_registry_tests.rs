use super::PeerRegistry;

/// A failure of the route a send used removes only that blob: the
/// route cached since stays (live finding CR2).
#[tokio::test]
async fn a_failed_blob_never_evicts_the_route_cached_since() {
    let mut registry = PeerRegistry::new(900, 3, 30);
    registry.cache_route("peer", b"old".to_vec());
    registry.cache_route("peer", b"new".to_vec());
    registry.invalidate_route_blob("peer", b"old");
    assert_eq!(registry.get_route("peer"), Some(&b"new"[..]));
    registry.invalidate_route_blob("peer", b"new");
    assert_eq!(registry.get_route("peer"), None);
}

/// A dead media route (another blob) leaves the peer's DM route.
#[tokio::test]
async fn a_dead_media_route_leaves_the_dm_route() {
    let mut registry = PeerRegistry::new(900, 3, 30);
    registry.cache_route("peer", b"dm".to_vec());
    assert!(registry.invalidate_blobs(&[b"media".to_vec()]).is_empty());
    assert_eq!(registry.get_route("peer"), Some(&b"dm"[..]));
}
