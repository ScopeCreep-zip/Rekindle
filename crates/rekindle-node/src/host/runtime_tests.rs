//! The host over a temp root with no node (plan D3.1).

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::*;
use crate::host::identity;
use rekindle_ipc::{Caller, ConnId, HostApi};

fn temp_root(dir: &tempfile::TempDir) -> DataRoot {
    let data = dir.path().join("data");
    DataRoot {
        config: dir.path().join("config"),
        logs: dir.path().join("logs"),
        state: data.join("state"),
        data,
    }
}

async fn offline_host(dir: &tempfile::TempDir) -> Arc<Host> {
    Host::start(HostConfig {
        root: claim(temp_root(dir)).unwrap(),
        stores: HostStores::default(),
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: std::sync::Arc::new(rekindle_utils::SystemClock),
        rng: std::sync::Arc::new(rekindle_utils::OsRandom),
    })
    .await
    .unwrap()
}

fn status_of(response: &IpcResponse) -> Option<u32> {
    match response {
        IpcResponse::Error { code, .. } => Some(*code),
        _ => None,
    }
}

/// The shell's login installs the identity in the host, so a request
/// reaches the same identity; its logout takes it out again.
#[tokio::test]
async fn an_installed_identity_is_the_hosts_until_released() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    assert_eq!(host.mode(), HostMode::Shell);

    let created = identity::create(host.root(), &host.identity_stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    let installed = host.install(created).await.unwrap();
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Operational);
    assert_eq!(
        host.ctx.require_owner().ok().as_deref(),
        Some(installed.public_key.as_str())
    );
    let shown = host.call(Caller::shell(), IpcRequest::IdentityShow).await;
    assert!(status_of(&shown).is_none(), "{shown:?}");

    host.release_identity().await;
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Locked);
    assert!(host.ctx.session.read().is_none());
    assert!(host.ctx.db.current().is_err());
}

/// In the shell the window owns the identity lifecycle and the process: a
/// bus client cannot lock, log out, create or shut down from under it.
#[tokio::test]
async fn the_shell_refuses_the_requests_the_window_owns() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let created = identity::create(host.root(), &host.identity_stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    host.install(created).await.unwrap();
    let bus = Caller {
        conn: ConnId::Bus(7),
        clearance: SecurityLevel::Admin,
        verified_name: None,
        static_key: None,
    };
    for request in [IpcRequest::Lock, IpcRequest::Logout, IpcRequest::Shutdown] {
        let answer = host.call(bus.clone(), request).await;
        assert_eq!(status_of(&answer), Some(409), "{answer:?}");
    }
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Operational);
}

/// The window's own identity calls are not refused: the refusal guards the
/// window against bus clients, not against itself.
#[tokio::test]
async fn the_shell_does_not_refuse_its_own_calls() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let created = identity::create(host.root(), &host.identity_stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    host.install(created).await.unwrap();
    let answer = host.call(Caller::shell(), IpcRequest::Lock).await;
    assert!(status_of(&answer).is_none(), "{answer:?}");
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Locked);
}

/// One host per data root: a second claim of a held root is refused.
#[tokio::test]
async fn a_held_root_cannot_be_claimed_again() {
    let dir = tempfile::tempdir().unwrap();
    let _host = offline_host(&dir).await;
    assert!(claim(temp_root(&dir)).is_err());
}
