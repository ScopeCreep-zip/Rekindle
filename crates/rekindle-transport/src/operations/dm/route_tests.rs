//! Plan RV1g B1 on the 1:1 path: a peer route Veilid declared dead
//! (`RouteChange.dead_remote`, `ping_validator.rs:740-744`) never reaches
//! the route port again from a 1:1 send, even when a cache write that took
//! no verdict (a presence watch's subkey 6, a request's or an accept's
//! route) put the blob back: `route::import` refuses it and forgets it.
//!
//! Fails when `route::import` imports through the port directly
//! (`node.routes().import(blob)`) instead of `TransportNode::import_route`.

use std::sync::Arc;

use rekindle_records::port::{
    AppCallId, AppMessagePort, PeerTarget, PortError, PortSet, RecordPort, RecordStore,
};
use rekindle_types::config::{SafetyProfile, TransportConfig};

use super::{cache, cached, cached_route};
use crate::broadcast::node::TransportNode;
use crate::liveness::receipt_tests::OneRoute;

struct NoRecords;

impl RecordStore for NoRecords {
    fn start_session(&self) -> Result<Arc<dyn RecordPort>, PortError> {
        Err(PortError::Routing("no records here".into()))
    }
}

struct NoMessages;

#[async_trait::async_trait]
impl AppMessagePort for NoMessages {
    async fn send(
        &self,
        _target: &PeerTarget,
        _profile: &SafetyProfile,
        _message: Vec<u8>,
    ) -> Result<(), PortError> {
        Err(PortError::Routing("no messages here".into()))
    }

    async fn call(
        &self,
        _target: &PeerTarget,
        _profile: &SafetyProfile,
        _message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        Err(PortError::Routing("no calls here".into()))
    }

    async fn reply(&self, _call_id: AppCallId, _message: Vec<u8>) -> Result<(), PortError> {
        Ok(())
    }
}

fn node() -> (Arc<OneRoute>, TransportNode) {
    let routes = OneRoute::new();
    let ports = PortSet {
        records: Arc::new(NoRecords),
        routes: routes.clone(),
        messages: Arc::new(NoMessages),
        lifecycle: None,
    };
    let session = Arc::new(parking_lot::RwLock::new(None));
    let node = TransportNode::adopt_outbound(TransportConfig::default(), ports, session);
    (routes, node)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_one_to_one_send_never_imports_a_route_veilid_declared_dead() {
    let (routes, node) = node();
    // A live blob reaches the port (whose import fails, so it is forgotten).
    cache(&node, "ola", b"live".to_vec());
    assert!(cached_route(&node, "ola").is_none());
    assert_eq!(routes.imports(), 1, "a live blob is imported");
    assert_eq!(
        cached(&node, "ola"),
        None,
        "a blob that did not import is forgotten"
    );

    // Veilid's remote route test declared Ola's route dead; a cache write
    // that took no verdict put it back.
    node.liveness().declare_dead_by_veilid(&[b"dead".to_vec()]);
    cache(&node, "ola", b"dead".to_vec());
    assert!(
        cached_route(&node, "ola").is_none(),
        "no target for a dead route"
    );
    assert_eq!(routes.imports(), 1, "the dead blob never reached the port");
    assert_eq!(cached(&node, "ola"), None, "the dead blob is forgotten");
}
