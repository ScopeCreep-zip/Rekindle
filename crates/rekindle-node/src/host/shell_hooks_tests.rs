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
    fn own_gossip_signed(&self, _community_id: &str, _sender: &str, _dedup_key: &str) {
        self.0.lock().push("own gossip".into());
    }
    fn community_left(&self, _community_id: &str) {}
    fn spawn_departure_rotation(&self, _community_id: &str, departed: &str) {
        self.0.lock().push(format!("rotation {departed}"));
    }
    fn require_permission(&self, _community_id: &str, _perm_bits: u64) -> Result<(), String> {
        Ok(())
    }
    fn spawn_presence_poll_tick(&self, _community_id: &str) {}
    async fn presence_refresh(&self, _community_id: &str) {}

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

const KICKER: [u8; 32] = [0xbe; 32];
const KICKED: &str = "kicked-member";

/// A shell Host holding `c1`, where `KICKED` is a known member and
/// `@everyone` holds `everyone_perms` (with VIEW_CHANNELS: without it
/// `compute_permissions` grants nothing); the shell's recorder.
async fn kick_host(dir: &tempfile::TempDir, everyone_perms: u64) -> (Arc<Host>, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let host = offline_host_with(dir, Some(recorder.clone())).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut gov = rekindle_governance::state::GovernanceState::default();
    gov.roles.insert(
        rekindle_types::id::RoleId([0; 16]),
        rekindle_governance::state::RoleState {
            name: "@everyone".into(),
            permissions: everyone_perms,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: rekindle_governance::fixture::stamp(0),
        },
    );
    host.ctx.communities.set_governance_state("c1", gov);
    if let Some(c) = host.ctx.communities.map().write().get_mut("c1") {
        c.known_members.insert(KICKED.into());
    }
    (host, recorder)
}

/// The relayed `Kick` the desktop's gossip relay hands the Host (GI3).
async fn relay_kick(host: &Host) {
    let kick = rekindle_codec::community::envelope::ControlPayload::Kick {
        target_pseudonym: KICKED.into(),
    };
    let governance = crate::host::adapters::governance::HostGovernance::new(Arc::clone(&host.ctx));
    assert!(
        governance
            .on_membership_control("c1", &hex::encode(KICKER), &kick)
            .await
    );
}

fn knows_kicked(host: &Host) -> bool {
    host.ctx
        .communities
        .map()
        .read()
        .get("c1")
        .is_some_and(|c| c.known_members.contains(KICKED))
}

/// A relayed kick from a member without KICK_MEMBERS changes nothing: the
/// member stays and no rotation starts (gossip is unauthenticated as to
/// authority).
#[tokio::test]
async fn a_relayed_kick_without_authority_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (host, recorder) = kick_host(&dir, rekindle_types::permissions::VIEW_CHANNELS).await;
    relay_kick(&host).await;
    assert!(knows_kicked(&host));
    assert!(!recorder.0.lock().iter().any(|h| h.starts_with("rotation")));
    host.shutdown().await.unwrap();
}

/// An authorized relayed kick forgets the member and starts exactly one
/// departure rotation, through the Host to the shell's MEK stage.
#[tokio::test]
async fn an_authorized_relayed_kick_forgets_and_rotates_once() {
    let dir = tempfile::tempdir().unwrap();
    let (host, recorder) = kick_host(
        &dir,
        rekindle_types::permissions::VIEW_CHANNELS | rekindle_types::permissions::KICK_MEMBERS,
    )
    .await;
    relay_kick(&host).await;
    assert!(!knows_kicked(&host));
    let rotations: Vec<String> = (recorder.0.lock().iter())
        .filter(|h| h.starts_with("rotation"))
        .cloned()
        .collect();
    assert_eq!(rotations, [format!("rotation {KICKED}")]);
    host.shutdown().await.unwrap();
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
                passphrase: rekindle_test_fixtures::PASSPHRASE.into(),
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
        fn own_gossip_signed(&self, _community_id: &str, _sender: &str, _dedup_key: &str) {}
        fn community_left(&self, _community_id: &str) {}
        fn spawn_departure_rotation(&self, _community_id: &str, _departed: &str) {}
        fn require_permission(&self, _community_id: &str, _perm_bits: u64) -> Result<(), String> {
            Ok(())
        }
        fn spawn_presence_poll_tick(&self, _community_id: &str) {}
        async fn presence_refresh(&self, _community_id: &str) {}

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
                passphrase: rekindle_test_fixtures::PASSPHRASE.into(),
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
