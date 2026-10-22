//! The host over a temp root with no node (plan D3.1).

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::*;
use crate::host::fixture::temp_root;
use crate::host::shell_hooks;
use rekindle_ipc::{Caller, ConnId, HostApi};

#[path = "community_info_tests.rs"]
mod community_info;
#[path = "login_stage_tests.rs"]
mod login_stage;
#[path = "membership_tests.rs"]
mod membership;

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

/// A key reader over an empty cache: the test shells hold no keys.
fn empty_channel_keys() -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
    let cache = Arc::new(crate::daemon::mek_rotation::MekCacheAdapter::new(Arc::new(
        parking_lot::RwLock::new(rekindle_transport::crypto::mek::MekCache::new()),
    )));
    Arc::new(rekindle_mek_rotation::MekKeyProvider::new(
        cache.clone(),
        cache,
    ))
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
        _community_id: &str,
        _envelope: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) -> Result<(), String> {
        Ok(())
    }
    fn ensure_files_cache_open(&self, _community_id: &str) {}
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
        fn ensure_files_cache_open(&self, _community_id: &str) {}
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

/// One host per data root: a second claim of a held root is refused.
#[tokio::test]
async fn a_held_root_cannot_be_claimed_again() {
    let dir = tempfile::tempdir().unwrap();
    let _host = offline_host(&dir).await;
    assert!(claim(temp_root(&dir)).is_err());
}

/// An own governance write updates the warm cache at once (plan E3.1 B2):
/// our author set replaces the cached one and the other authors stay.
#[tokio::test]
async fn an_own_governance_write_updates_the_warm_cache() {
    use rekindle_governance_runtime::GovernanceRuntimeDeps;
    use rekindle_types::id::PseudonymKey;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let db = host.ctx.db.current().unwrap();
    let other = PseudonymKey([1; 32]);
    let me = PseudonymKey([2; 32]);
    let seeded = owner.clone();
    db.call(move |conn| {
        rekindle_db::repo::communities::insert_if_absent(
            conn,
            &seeded,
            &rekindle_db::repo::communities::NewCommunity {
                id: "c1",
                name: "C",
                my_role_ids: &[],
                joined_at: 1,
                dht_owner_keypair: None,
                my_pseudonym_key: "",
                mek_generation: 0,
                member_registry_key: None,
                my_subkey_index: None,
                governance_key: None,
            },
        )?;
        rekindle_db::repo::governance_cache::save(
            conn,
            &seeded,
            "c1",
            &[(PseudonymKey([1; 32]), Vec::new())],
            1,
        )
    })
    .await
    .unwrap();

    host.governance()
        .persist_own_governance_entries("c1", &me, &[]);
    let loaded = db
        .call(move |conn| rekindle_db::repo::governance_cache::load(conn, &owner))
        .await
        .unwrap();
    let authors: Vec<_> = loaded[0].1.iter().map(|(pk, _)| pk.clone()).collect();
    assert_eq!(authors, vec![other, me]);
    host.shutdown().await.unwrap();
}

/// Ann's community `c1` in the store with a member registry, and its row.
async fn seed_registry_community(host: &Host, owner: &str) {
    host.ctx.communities.map().write().insert(
        "c1".into(),
        crate::host::community::CommunityState {
            member_registry_key: Some("registry".into()),
            ..crate::host::community::CommunityState::new("c1")
        },
    );
    let owner = owner.to_string();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::communities::insert_if_absent(
                conn,
                &owner,
                &rekindle_db::repo::communities::NewCommunity {
                    id: "c1",
                    name: "C",
                    my_role_ids: &[],
                    joined_at: 1,
                    dht_owner_keypair: None,
                    my_pseudonym_key: "",
                    mek_generation: 0,
                    member_registry_key: Some("registry"),
                    my_subkey_index: None,
                    governance_key: None,
                },
            )
        })
        .await
        .unwrap();
}

/// Hydration recovers the stored registry owner keypair (plan E3.1 B2d):
/// a community whose store entry lacks it gets the vault's.
#[tokio::test]
async fn hydration_recovers_the_stored_registry_keypair() {
    use rekindle_governance_runtime::GovernanceRuntimeDeps;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    rekindle_vault::typed::community_keys::persist_registry_keypair(
        host.ctx.vault.lock().as_ref().unwrap(),
        "c1",
        "registry-keypair",
    );

    let governance = host.governance();
    assert_eq!(
        governance.list_missing_registry_keypairs(),
        vec!["c1".to_string()]
    );
    governance.recover_registry_keypair_from_keystore("c1");
    let stored = host.ctx.communities.map().read()["c1"]
        .registry_owner_keypair
        .clone();
    assert_eq!(stored.as_deref(), Some("registry-keypair"));
    assert!(governance.list_missing_registry_keypairs().is_empty());
    host.shutdown().await.unwrap();
}

/// The members a join discovered are stored (plan E3.1 B2d): known to the
/// community and in its `community_members` rows with their roles.
#[tokio::test]
async fn discovered_registry_members_are_persisted() {
    use rekindle_governance_runtime::{DiscoveredMember, GovernanceRuntimeDeps};

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let bea = rekindle_types::presence::MemberPresence {
        pseudonym_key: rekindle_types::id::PseudonymKey([7; 32]),
        display_name: Some("Bea".into()),
        ..Default::default()
    };
    host.governance().persist_discovered_registry_members(
        "c1",
        vec![DiscoveredMember {
            segment_index: 0,
            slot_index: 3,
            presence: bea,
            role_ids: vec![0, 2],
        }],
    );

    let bea_hex = hex::encode([7; 32]);
    assert!(host.ctx.communities.known_members("c1").contains(&bea_hex));
    let roster = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::members::roster(conn, &owner, "c1"))
        .await
        .unwrap();
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].pseudonym_key, bea_hex);
    assert_eq!(roster[0].display_name.as_deref(), Some("Bea"));
    assert_eq!(roster[0].role_ids, vec![0, 2]);
    host.shutdown().await.unwrap();
}

#[path = "runtime_projection_tests.rs"]
mod projection;
