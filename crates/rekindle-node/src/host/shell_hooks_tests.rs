//! The Host's shell hooks: the stages a shell hears, the requests it refuses and
//! the gossip the Host routes through it (split from `runtime_tests.rs`).

use super::*;

/// The hooks a test shell records, in order.
#[derive(Default)]
struct Recorder(parking_lot::Mutex<Vec<String>>);

/// A key reader over an empty cache: the test shells hold no keys.
fn empty_channel_keys() -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
    crate::daemon::mek_rotation::provider_over(
        Arc::new(parking_lot::RwLock::new(
            rekindle_transport::crypto::mek::MekCache::new(),
        )),
        rekindle_vault::new_handle(),
    )
}

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
    fn send_to_mesh(
        &self,
        community_id: &str,
        _envelope: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) -> Result<(), String> {
        self.0.lock().push(format!("mesh {community_id}"));
        Ok(())
    }
    fn community_left(&self, _community_id: &str) {}
    fn spawn_departure_rotation(&self, _community_id: &str, _departed: &str) {}
    fn require_permission(&self, _community_id: &str, _perm_bits: u64) -> Result<(), String> {
        Ok(())
    }
    fn spawn_presence_poll_tick(&self, _community_id: &str) {}

    fn spawn_community_mek_recovery(&self, _community_id: &str, _requester_pseudonym: &str) {}

    fn push_onboarding_complete_to_sync(&self, _community_id: &str) {}

    fn on_mek_installed(
        &self,
        _community_id: &str,
        _scope: rekindle_types::channel_keys::KeyScope,
        _key_bytes: [u8; 32],
        _generation: u64,
    ) {
    }

    fn channel_keys(&self) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        empty_channel_keys()
    }

    async fn community_records_ready(
        &self,
        _community_id: &str,
        _leases: rekindle_records::lease::CommunityLeases,
    ) {
    }

    fn on_governance_projected(
        &self,
        _community_id: &str,
        _projected: &crate::host::adapters::governance::projection::GovernanceProjected,
    ) {
    }
}

/// In the shell the gossip worker does not run, so the Host's own gossip
/// goes to the shell's mesh; queued on the worker it was dropped.
#[tokio::test]
async fn the_host_gossips_through_the_shell_mesh() {
    use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
    let dir = tempfile::tempdir().unwrap();
    let recorder = Arc::new(Recorder::default());
    let host = offline_host_with(&dir, Some(recorder.clone())).await;
    let notice = CommunityEnvelope::Control(ControlPayload::Kick {
        target_pseudonym: "ab".repeat(32),
    });
    crate::host::adapters::governance::HostGovernance::new(Arc::clone(&host.ctx))
        .send_to_mesh_impl("community-1", &notice);
    host.shutdown().await.unwrap();
    assert!(
        recorder.0.lock().iter().any(|e| e == "mesh community-1"),
        "{:?}",
        recorder.0.lock()
    );
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
        fn send_to_mesh(
            &self,
            _community_id: &str,
            _envelope: &rekindle_codec::community::envelope::CommunityEnvelope,
        ) -> Result<(), String> {
            Ok(())
        }
        fn community_left(&self, _community_id: &str) {}
        fn spawn_departure_rotation(&self, _community_id: &str, _departed: &str) {}
        fn require_permission(&self, _community_id: &str, _perm_bits: u64) -> Result<(), String> {
            Ok(())
        }
        fn spawn_presence_poll_tick(&self, _community_id: &str) {}

        fn spawn_community_mek_recovery(&self, _community_id: &str, _requester_pseudonym: &str) {}

        fn push_onboarding_complete_to_sync(&self, _community_id: &str) {}

        fn on_mek_installed(
            &self,
            _community_id: &str,
            _scope: rekindle_types::channel_keys::KeyScope,
            _key_bytes: [u8; 32],
            _generation: u64,
        ) {
        }

        fn channel_keys(&self) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
            empty_channel_keys()
        }

        async fn community_records_ready(
            &self,
            _community_id: &str,
            _leases: rekindle_records::lease::CommunityLeases,
        ) {
        }

        fn on_governance_projected(
            &self,
            _community_id: &str,
            _projected: &crate::host::adapters::governance::projection::GovernanceProjected,
        ) {
        }
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

/// A shell-mode stop does not wait out the worker deadline: the panic
/// teardown worker ends when the stop is requested (Linux finding: every
/// duress exit logged `workers still running ... running=1`, 2 s of 2.5 s).
#[tokio::test]
async fn a_shell_host_stops_before_the_worker_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host_with(&dir, Some(Arc::new(Recorder::default()))).await;
    let started = std::time::Instant::now();
    host.shutdown().await.unwrap();
    assert!(
        started.elapsed() < crate::daemon::shutdown::WORKER_STOP_DEADLINE,
        "stop took {:?}",
        started.elapsed()
    );
}
