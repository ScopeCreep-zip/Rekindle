//! The host over a temp root with no node (plan D3.1).

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::*;
use crate::host::fixture::temp_root;
use crate::host::shell_hooks;
use rekindle_ipc::{Caller, ConnId, HostApi};

async fn offline_host(dir: &tempfile::TempDir) -> Arc<Host> {
    offline_host_with(dir, None).await
}

async fn offline_host_with(
    dir: &tempfile::TempDir,
    shell: Option<Arc<dyn shell_hooks::ShellHooks>>,
) -> Arc<Host> {
    Host::start(HostConfig {
        root: claim(temp_root(dir)).unwrap(),
        stores: HostStores::default(),
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: std::sync::Arc::new(rekindle_utils::SystemClock),
        rng: std::sync::Arc::new(rekindle_utils::OsRandom),
        idle: std::sync::Arc::new(rekindle_idle::OsIdle),
        shell,
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

/// Create Ann through the window, which unlocks her.
async fn create_ann(host: &Host) -> rekindle_types::dto::identity::IdentityView {
    let answer = host
        .call(
            Caller::shell(),
            IpcRequest::IdentityCreate {
                display_name: Some("Ann".into()),
                passphrase: "pass".into(),
            },
        )
        .await;
    match answer {
        IpcResponse::Ok { data, .. } => serde_json::from_value(data).unwrap(),
        other => panic!("create refused: {other:?}"),
    }
}

/// One create makes the identity and unlocks it (plan E1.1), offline: a
/// request then reaches that identity, and a logout takes it out again.
#[tokio::test]
async fn a_create_unlocks_until_logout() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    assert_eq!(host.mode(), HostMode::Shell);

    let view = create_ann(&host).await;
    assert_eq!(view.display_name, "Ann");
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Operational);
    assert_eq!(
        host.ctx.require_owner().ok().as_deref(),
        Some(view.public_key.as_str())
    );
    let shown = host.call(Caller::shell(), IpcRequest::IdentityShow).await;
    let IpcResponse::Ok { data, .. } = shown else {
        panic!("{shown:?}")
    };
    let shown: Option<rekindle_types::dto::identity::IdentityShown> =
        serde_json::from_value(data).unwrap();
    assert_eq!(shown.map(|s| s.public_key), Some(view.public_key));

    let out = host.call(Caller::shell(), IpcRequest::Logout).await;
    assert!(status_of(&out).is_none(), "{out:?}");
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Locked);
    assert!(host.ctx.session.read().is_none());
    assert!(host.ctx.db.current().is_err());
    let shown = host.call(Caller::shell(), IpcRequest::IdentityShow).await;
    assert!(matches!(shown, IpcResponse::Ok { data, .. } if data.is_null()));
}

/// The hooks a test shell records, in order.
#[derive(Default)]
struct Recorder(parking_lot::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl shell_hooks::ShellHooks for Recorder {
    async fn unlocked(
        &self,
        _view: &shell_hooks::UnlockedView,
        _scope: &Arc<rekindle_lifecycle::SessionScope>,
    ) -> Result<(), String> {
        self.0.lock().push("unlocked".into());
        Ok(())
    }
    fn own_records(&self, _keys: &shell_hooks::OwnRecordKeys) {
        self.0.lock().push("own records".into());
    }
    async fn ending(&self) {
        self.0.lock().push("ending".into());
    }
    async fn ended(&self, reason: shell_hooks::EndReason) {
        self.0.lock().push(format!("ended {reason:?}"));
    }
}

/// The Host calls the shell at its fixed points: after its own stages,
/// before the scope is cancelled, and once the identity is out.
#[tokio::test]
async fn the_shell_hears_the_unlock_and_its_end_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Arc::new(Recorder::default());
    let host = offline_host_with(&dir, Some(recorder.clone())).await;
    let view = create_ann(&host).await;
    let lock = host.call(Caller::shell(), IpcRequest::Lock).await;
    assert!(status_of(&lock).is_none(), "{lock:?}");
    let again = host
        .call(
            Caller::shell(),
            IpcRequest::Unlock {
                identity: view.public_key,
                passphrase: "pass".into(),
            },
        )
        .await;
    assert!(status_of(&again).is_none(), "{again:?}");
    host.shutdown().await.unwrap();
    assert_eq!(
        *recorder.0.lock(),
        [
            "unlocked",
            "ending",
            "ended Lock",
            "unlocked",
            "ending",
            "ended Exit",
        ]
    );
}

/// A shell stage that cannot start aborts the unlock: the identity is out
/// and the host is Locked.
#[tokio::test]
async fn a_failed_shell_stage_leaves_nothing_unlocked() {
    struct Refuses;
    #[async_trait::async_trait]
    impl shell_hooks::ShellHooks for Refuses {
        async fn unlocked(
            &self,
            _view: &shell_hooks::UnlockedView,
            _scope: &Arc<rekindle_lifecycle::SessionScope>,
        ) -> Result<(), String> {
            Err("no".into())
        }
        fn own_records(&self, _keys: &shell_hooks::OwnRecordKeys) {}
        async fn ending(&self) {}
        async fn ended(&self, _reason: shell_hooks::EndReason) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host_with(&dir, Some(Arc::new(Refuses))).await;
    let answer = host
        .call(
            Caller::shell(),
            IpcRequest::IdentityCreate {
                display_name: None,
                passphrase: "pass".into(),
            },
        )
        .await;
    assert_eq!(status_of(&answer), Some(500), "{answer:?}");
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Locked);
    assert!(host.ctx.session.read().is_none());
    assert!(host.ctx.db.current().is_err());
    assert!(host.ctx.unlock_scope.read().is_none());
}

/// In the shell the window owns the identity lifecycle and the process: a
/// bus client cannot lock, log out, create or shut down from under it.
#[tokio::test]
async fn the_shell_refuses_the_requests_the_window_owns() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
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
    create_ann(&host).await;
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
