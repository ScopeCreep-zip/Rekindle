//! The desktop's create and unlock through the Host (plan E1.1): the window's
//! Host calls (`host_call::call`) against an Offline Host running the GUI's
//! `ShellSession` hooks.
//!
//! Real vaults, real SQLCipher databases and real Ed25519 keys in a temp
//! data root: no mocking.

use std::sync::Arc;

use rekindle_db::paths::DataRoot;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_lib::host_call;
use rekindle_lib::state::{AppState, SharedState, UserStatus};
use rekindle_types::dto::identity::IdentityView;
use rekindle_types::dto::CommandError;

/// A data root under `dir`, laid out like the real one, claimed by an
/// in-process host with no node and the GUI's hooks, and the state that
/// shares the host's identity slots.
async fn test_env(dir: &std::path::Path) -> (DataRoot, SharedState) {
    let data = dir.join("data");
    let root = DataRoot {
        config: dir.join("config"),
        logs: dir.join("logs"),
        state: data.join("state"),
        data,
    };
    let state: SharedState = Arc::new(AppState::default());
    let host = rekindle_node::host::Host::start(rekindle_node::host::HostConfig {
        root: rekindle_node::host::claim(root.clone()).unwrap(),
        stores: state.host_stores(),
        transport: rekindle_node::host::TransportSource::Offline,
        serve_bus: false,
        clock: Arc::new(rekindle_utils::SystemClock),
        rng: Arc::new(rekindle_utils::OsRandom),
        shell: Some(rekindle_lib::ShellSession::new(Arc::clone(&state), None) as _),
    })
    .await
    .unwrap();
    state.host.set(host);
    (root, state)
}

async fn create(
    state: &SharedState,
    passphrase: &str,
    name: Option<&str>,
) -> Result<IdentityView, CommandError> {
    host_call::call(
        state,
        IpcRequest::IdentityCreate {
            display_name: name.map(str::to_owned),
            passphrase: passphrase.into(),
        },
    )
    .await
    .map(|done| done.value)
}

async fn login(
    state: &SharedState,
    public_key: &str,
    passphrase: &str,
) -> Result<IdentityView, CommandError> {
    host_call::call(
        state,
        IpcRequest::Unlock {
            identity: public_key.into(),
            passphrase: passphrase.into(),
        },
    )
    .await
    .map(|done| done.value)
}

async fn logout(state: &SharedState) {
    host_call::call::<serde_json::Value>(state, IpcRequest::Logout)
        .await
        .expect("logout");
}

fn code(error: &CommandError) -> u32 {
    match error {
        CommandError::Failed { code, .. } => *code,
        CommandError::IntentRequired(review) => panic!("held for review: {review:?}"),
    }
}

// ── Create Identity ──────────────────────────────────────────────────

#[tokio::test]
async fn create_identity_writes_the_vault_database_and_registry_and_unlocks() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let created = create(&state, "test-passphrase", Some("Alice"))
        .await
        .expect("create should succeed offline");

    assert_eq!(created.display_name, "Alice");
    {
        let identity = state.identity.read();
        let id = identity.as_ref().expect("identity should be set");
        assert_eq!(id.public_key, created.public_key);
        assert_eq!(id.display_name, "Alice");
        assert_eq!(id.status, UserStatus::Online);
    }
    assert_eq!(
        state.lifecycle.state(),
        rekindle_lifecycle::LifecycleState::Operational
    );

    // The identity row is in the identity's own database.
    let pk = created.public_key.clone();
    let name = state
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::identity::login_row(conn, &pk))
        .await
        .unwrap()
        .map(|(name, _)| name);
    assert_eq!(name.as_deref(), Some("Alice"));

    // The unlock reached the shell: the vault (the friend authority and the
    // ratchet are the Host's, plans E2.1, E2.3).
    assert!(state.keystore.lock().is_some());
    assert_ne!(*state.identity_secret.lock(), None);

    // Files and registry.
    assert!(root.identity_vault(&created.public_key).exists());
    assert!(root.identity_db(&created.public_key).exists());
    let listed = rekindle_db::registry::load(&root.registry()).unwrap();
    assert_eq!(
        listed.get(&created.public_key).unwrap().display_name,
        "Alice"
    );
}

#[tokio::test]
async fn create_identity_without_a_name_uses_the_fallback() {
    let dir = tempfile::TempDir::new().unwrap();
    let (_root, state) = test_env(dir.path()).await;

    let created = create(&state, "pass", None).await.unwrap();

    assert!(
        created.display_name.starts_with("User_"),
        "expected fallback display name, got: {}",
        created.display_name
    );
    assert_eq!(created.display_name.len(), 13); // "User_" + 8 hex chars
}

#[tokio::test]
async fn a_blank_name_is_refused_not_taken_for_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let refused = create(&state, "pass", Some("   ")).await.unwrap_err();

    assert_eq!(code(&refused), 400);
    assert!(rekindle_db::registry::load(&root.registry())
        .unwrap()
        .identities
        .is_empty());
}

// ── Login ────────────────────────────────────────────────────────────

#[tokio::test]
async fn login_after_logout_restores_the_same_keypair() {
    let dir = tempfile::TempDir::new().unwrap();
    let (_root, state) = test_env(dir.path()).await;

    let created = create(&state, "roundtrip-pass", Some("Charlie"))
        .await
        .unwrap();
    let create_secret = state.identity_secret.lock().expect("a secret");

    logout(&state).await;
    assert!(state.identity.read().is_none());
    assert!(state.keystore.lock().is_none());
    assert!(state.db.current().is_err(), "logout closes the database");
    assert_eq!(*state.identity_secret.lock(), None);

    let login = login(&state, &created.public_key, "roundtrip-pass")
        .await
        .expect("login should succeed with correct passphrase");

    assert_eq!(login.public_key, created.public_key);
    assert_eq!(login.display_name, "Charlie");
    assert_eq!(*state.identity_secret.lock(), Some(create_secret));
    let identity = state.identity.read();
    let id = identity.as_ref().expect("identity should be restored");
    assert_eq!(id.public_key, created.public_key);
    assert_eq!(id.status, UserStatus::Online);
    assert!(state.keystore.lock().is_some());
}

#[tokio::test]
async fn login_fails_with_wrong_passphrase_and_installs_nothing() {
    let dir = tempfile::TempDir::new().unwrap();
    let (_root, state) = test_env(dir.path()).await;

    let created = create(&state, "correct-pass", Some("Dave")).await.unwrap();
    logout(&state).await;

    let err = login(&state, &created.public_key, "wrong-pass")
        .await
        .expect_err("login with wrong passphrase should fail");

    assert_eq!(code(&err), 401, "{err:?}");
    assert!(state.identity.read().is_none());
    assert!(state.keystore.lock().is_none());
    assert!(state.db.current().is_err());
    assert_eq!(
        state.lifecycle.state(),
        rekindle_lifecycle::LifecycleState::Locked
    );
}

#[tokio::test]
async fn login_fails_with_no_identity() {
    let dir = tempfile::TempDir::new().unwrap();
    let (_root, state) = test_env(dir.path()).await;

    let err = login(&state, &"ab".repeat(32), "any-pass")
        .await
        .expect_err("login with no identity should fail");

    assert_eq!(code(&err), 404, "{err:?}");
}

#[tokio::test]
async fn two_identities_coexist_and_unlock_one_at_a_time() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let first = create(&state, "pass-one", Some("First")).await.unwrap();
    logout(&state).await;
    let second = create(&state, "pass-two", Some("Second")).await.unwrap();
    logout(&state).await;

    let listed = rekindle_db::registry::load(&root.registry()).unwrap();
    assert_eq!(listed.identities.len(), 2);

    let login_first = login(&state, &first.public_key, "pass-one").await.unwrap();
    assert_eq!(login_first.display_name, "First");
    logout(&state).await;

    let err = login(&state, &second.public_key, "pass-one")
        .await
        .expect_err("each identity has its own passphrase");
    assert_eq!(code(&err), 401, "{err:?}");
    let login_second = login(&state, &second.public_key, "pass-two").await.unwrap();
    assert_eq!(login_second.display_name, "Second");
}
