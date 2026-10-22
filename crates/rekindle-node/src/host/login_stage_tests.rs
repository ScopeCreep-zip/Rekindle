//! The communities' login network stage (plan E3.1 C2a) is a task of the
//! unlock's `SessionScope`: a lock cancels it, waiting for the network or
//! not.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};

use super::{create_ann, offline_host};
use crate::host::adapters::governance::login_stage;

#[tokio::test]
async fn the_network_stage_runs_in_the_unlock_scope_and_stops_at_lock() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let scope = host
        .ctx
        .unlock_scope
        .read()
        .clone()
        .expect("an unlocked identity has an unlock scope");
    // The network never comes up: the stage waits for it.
    let (_ready_tx, ready) = tokio::sync::watch::channel(false);
    login_stage::start(host.governance(), ready, &scope).expect("the scope is open");
    assert!(
        scope
            .running()
            .iter()
            .any(|(name, _)| *name == login_stage::STAGE_TASK),
        "the stage is a task of the unlock scope"
    );

    let lock = host.call(Caller::shell(), IpcRequest::Lock).await;
    assert!(matches!(lock, IpcResponse::Ok { .. }), "{lock:?}");
    assert!(scope.is_closed(), "the lock closed the unlock scope");
    assert!(
        !scope
            .running()
            .iter()
            .any(|(name, _)| *name == login_stage::STAGE_TASK),
        "the lock cancelled the stage"
    );
    host.shutdown().await.expect("the host stops");
}
