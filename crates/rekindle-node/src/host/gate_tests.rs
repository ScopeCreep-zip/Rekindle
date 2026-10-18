//! The one intent gate, through `HostApi::call` on an offline host
//! (plan D3.3).

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::intent::{IntentClass, IntentProof, IntentReview, ProofRequired};

use crate::host::api::{Caller, ConnId, HostApi};
use crate::host::{claim, identity, Host, HostConfig, HostStores, TransportSource};

fn temp_root(dir: &tempfile::TempDir) -> rekindle_db::paths::DataRoot {
    let data = dir.path().join("data");
    rekindle_db::paths::DataRoot {
        config: dir.path().join("config"),
        logs: dir.path().join("logs"),
        state: data.join("state"),
        data,
    }
}

fn bus(conn: u64) -> Caller {
    Caller {
        conn: ConnId::Bus(conn),
        clearance: SecurityLevel::Admin,
        verified_name: None,
        static_key: None,
    }
}

/// A host with one identity, created and locked again.
async fn host_with_identity(dir: &tempfile::TempDir) -> (std::sync::Arc<Host>, String) {
    let host = Host::start(HostConfig {
        root: claim(temp_root(dir)).unwrap(),
        stores: HostStores::default(),
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: std::sync::Arc::new(rekindle_utils::SystemClock),
        rng: std::sync::Arc::new(rekindle_utils::OsRandom),
    })
    .await
    .unwrap();
    let created = identity::create(host.root(), &host.identity_stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    let public_key = created.public_key.clone();
    drop(created);
    identity::lock(&host.identity_stores()).await;
    (host, public_key)
}

fn review(answer: IpcResponse) -> IntentReview {
    match answer {
        IpcResponse::IntentRequired(review) => review,
        other => panic!("expected a review, got {other:?}"),
    }
}

fn code(answer: &IpcResponse) -> Option<u32> {
    match answer {
        IpcResponse::Error { code, .. } => Some(*code),
        _ => None,
    }
}

/// Deleting an identity is Protected: held for its passphrase, refused on
/// a wrong one (still held), run once on the right one, its answer
/// repeated for the same id, and unknown to another connection.
#[tokio::test]
async fn a_protected_delete_runs_once_on_its_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let (host, public_key) = host_with_identity(&dir).await;
    let delete = IpcRequest::IdentityDelete {
        identity: public_key.clone(),
    };
    let held = review(host.call(bus(1), delete).await);
    assert_eq!(held.class, IntentClass::Protected);
    assert_eq!(held.proof, ProofRequired::Passphrase);

    let commit = |passphrase: &str| IpcRequest::IntentCommit {
        intent_id: held.intent_id,
        proof: IntentProof::Passphrase(passphrase.to_string().into()),
    };
    assert_eq!(code(&host.call(bus(1), commit("wrong")).await), Some(403));
    assert_eq!(code(&host.call(bus(2), commit("pass")).await), Some(404));
    let ran = host.call(bus(1), commit("pass")).await;
    assert!(
        matches!(
            ran,
            IpcResponse::Ok {
                reversibility: Some(_),
                ..
            }
        ),
        "{ran:?}"
    );
    assert!(identity::list(host.root()).unwrap().is_empty());
    let again = host.call(bus(1), commit("pass")).await;
    assert!(matches!(again, IpcResponse::Ok { .. }), "{again:?}");
}

/// A cancelled action cannot be committed; a closed connection's actions
/// are dropped.
#[tokio::test]
async fn cancelled_and_disconnected_actions_are_gone() {
    let dir = tempfile::tempdir().unwrap();
    let (host, public_key) = host_with_identity(&dir).await;
    let delete = || IpcRequest::IdentityDelete {
        identity: public_key.clone(),
    };
    let first = review(host.call(bus(1), delete()).await);
    let cancelled = host
        .call(
            bus(1),
            IpcRequest::IntentCancel {
                intent_id: first.intent_id,
            },
        )
        .await;
    assert!(matches!(cancelled, IpcResponse::Ok { .. }));
    let commit = |id| IpcRequest::IntentCommit {
        intent_id: id,
        proof: IntentProof::Passphrase("pass".to_string().into()),
    };
    assert_eq!(
        code(&host.call(bus(1), commit(first.intent_id)).await),
        Some(404)
    );

    let second = review(host.call(bus(3), delete()).await);
    crate::host::gate::clear_connection(&host.ctx.intents, 3);
    assert_eq!(
        code(&host.call(bus(3), commit(second.intent_id)).await),
        Some(404)
    );
    assert_eq!(identity::list(host.root()).unwrap().len(), 1);
}
