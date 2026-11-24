//! Plan RV1g B1, B3 over the ports: a peer blob Veilid declared dead is
//! refused at import as
//! `RouteUnusable` until a strictly newer read of the peer's record or the
//! failure cache's expiry.

use std::sync::Arc;

use rekindle_route::liveness::FAILURE_CACHE;

use super::receipt_tests::OneRoute;
use super::Liveness;
use crate::broadcast::node::routes::import_live;
use crate::error::TransportError;

fn liveness() -> (Arc<OneRoute>, Arc<Liveness>) {
    let routes = OneRoute::new();
    let live = Liveness::new(Arc::new(rekindle_utils::OsRandom), routes.clone(), true);
    (routes, live)
}

/// `true` when the import reached the port (whose import always fails
/// with `RouteImportFailed`), `false` when it was refused as dead.
fn reaches_port(routes: &OneRoute, live: &Liveness, blob: &[u8]) -> bool {
    match import_live(routes, live, blob) {
        Err(TransportError::RouteImportFailed { .. }) => true,
        Err(TransportError::RouteUnusable { reason, .. }) => {
            assert_eq!(reason, "declared dead");
            false
        }
        other => panic!("unexpected import result {other:?}"),
    }
}

#[test]
fn a_blob_veilid_declared_dead_is_refused_until_a_higher_seq_read() {
    let (routes, live) = liveness();
    assert!(reaches_port(&routes, &live, b"A"));
    live.declare_dead_by_veilid(&[b"A".to_vec()]);
    assert!(!reaches_port(&routes, &live, b"A"), "refused while dead");
    assert!(reaches_port(&routes, &live, b"B"), "another blob is not");
    live.route_read_at(b"A", 4);
    live.route_read_at(b"A", 4);
    assert!(!reaches_port(&routes, &live, b"A"), "the same record");
    live.route_read_at(b"A", 5);
    assert!(reaches_port(&routes, &live, b"A"), "republished");
}

#[tokio::test(start_paused = true)]
async fn a_dead_blob_is_accepted_again_after_the_failure_cache() {
    let (routes, live) = liveness();
    live.declare_dead_by_veilid(&[b"A".to_vec()]);
    tokio::time::advance(FAILURE_CACHE.saturating_sub(std::time::Duration::from_secs(1))).await;
    assert!(!reaches_port(&routes, &live, b"A"));
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    assert!(reaches_port(&routes, &live, b"A"), "expired");
}
