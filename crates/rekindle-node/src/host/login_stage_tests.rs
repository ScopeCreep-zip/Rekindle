//! The communities' unlock stages: the login network stage (plan E3.1
//! C2a) is a task of the unlock's `SessionScope`, so a lock cancels it,
//! waiting for the network or not; the MEK stage (C2c) reloads the stored
//! keys.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};

use rekindle_types::channel_keys::KeyScope;

use super::{create_ann, offline_host, seed_registry_community};
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

/// An unlock reloads the stored MEKs into the Host's cache (plan E3.1
/// C2c): a key a JoinAccepted installed is there again after a relock.
#[tokio::test]
async fn an_unlock_restores_the_stored_meks() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let key = rekindle_transport::crypto::mek::Mek::from_bytes([7; 32], 3);
    let payload = rekindle_codec::community::envelope::ControlPayload::JoinAccepted {
        mek_encrypted: key.to_wire_bytes(),
        mek_generation: 3,
        members: Vec::new(),
        member_registry_key: Some("registry-2".into()),
        slot_index: None,
        wrapped_slot_seed: None,
    };
    assert!(
        host.governance()
            .on_membership_control("c1", "bob", &payload)
            .await
    );

    let lock = host.call(Caller::shell(), IpcRequest::Lock).await;
    assert!(matches!(lock, IpcResponse::Ok { .. }), "{lock:?}");
    assert!(
        host.ctx
            .mek_cache
            .read()
            .current("c1", KeyScope::Community)
            .is_none(),
        "the lock dropped the cached key"
    );
    let unlock = host
        .call(
            Caller::shell(),
            IpcRequest::Unlock {
                identity: ann.public_key,
                passphrase: "pass".into(),
            },
        )
        .await;
    assert!(matches!(unlock, IpcResponse::Ok { .. }), "{unlock:?}");
    let cached = host
        .ctx
        .mek_cache
        .read()
        .current("c1", KeyScope::Community)
        .map(|mek| (*mek.as_bytes(), mek.generation()));
    assert_eq!(cached, Some(([7; 32], 3)), "the unlock reloaded the key");
    host.shutdown().await.expect("the host stops");
}
