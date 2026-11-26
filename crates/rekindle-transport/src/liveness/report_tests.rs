//! Plan RV2, evidence source 7 (caller side): a re-read of a blob Veilid
//! declared dead is reported once per blob, and the blob stays refused at
//! import (the suspect probe goes through the same refusal).

use std::sync::Arc;

use super::super::receipt_tests::OneRoute;
use super::super::Liveness;
use super::blob_hash;
use crate::error::TransportError;

#[test]
fn a_dead_blob_is_reported_once_and_never_imported_again() {
    let routes = OneRoute::new();
    let live = Liveness::new(Arc::new(rekindle_utils::OsRandom), routes.clone(), true);
    assert_eq!(live.report_due("cat", b"B"), None, "not declared dead");
    live.declare_dead_by_veilid(&[b"B".to_vec()]);
    assert_eq!(live.report_due("cat", b"B"), Some(blob_hash(b"B")));
    assert_eq!(live.report_due("cat", b"B"), None, "one report per blob");
    assert!(
        matches!(
            crate::broadcast::node::routes::import_live(routes.as_ref(), &live, b"B"),
            Err(TransportError::RouteUnusable { .. })
        ),
        "the dead blob is not imported again"
    );
    live.declare_dead_by_veilid(&[b"C".to_vec()]);
    assert_eq!(
        live.report_due("cat", b"C"),
        Some(blob_hash(b"C")),
        "a new dead blob gets its own report"
    );
}
