use super::{MemberRouteCache, RouteVersion};

fn version(seq: u64, blob: u8) -> RouteVersion {
    RouteVersion {
        seq,
        blob: vec![blob],
    }
}

#[test]
fn device_records_order_their_own_versions() {
    let mut cache = MemberRouteCache::default();
    let a = version(12, 1);
    let b = version(2, 2);
    assert!(cache.verify("first", &a));
    assert!(cache.verify("second", &b));
    assert!(cache.set_usable("first", &a, true));
    assert!(cache.set_usable("second", &b, true));
    assert_eq!(cache.get("first"), Some(&a));
    assert_eq!(cache.get("second"), Some(&b));
    assert!(!cache.verify("first", &version(11, 3)));
    assert!(!cache.verify("first", &version(12, 3)));
    cache.retain(|record| record == "first");
    assert_eq!(cache.get("second"), None);
}

#[test]
fn a_dead_successor_keeps_its_watermark_against_stale_reads_and_probes() {
    let mut cache = MemberRouteCache::default();
    let initial = version(1, 1);
    let successor = version(2, 2);
    assert!(cache.verify("record", &initial));
    assert!(cache.set_usable("record", &initial, true));
    assert!(cache.verify("record", &successor));
    assert!(cache.set_usable("record", &successor, true));
    assert!(!cache.set_usable("record", &initial, false));
    assert_eq!(cache.get("record"), Some(&successor));
    cache.forget_blobs(|blob| blob == successor.blob);
    assert_eq!(cache.get("record"), None);
    assert!(cache.is_current("record", &successor));
    assert!(!cache.verify("record", &initial));
    assert!(!cache.set_usable("record", &initial, true));
    assert_eq!(cache.get("record"), None);
}
