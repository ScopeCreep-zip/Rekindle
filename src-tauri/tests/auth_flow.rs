//! Integration tests for the desktop's create and unlock (plan D1).
//!
//! Real vaults, real SQLCipher databases and real Ed25519 keys in a temp
//! data root: no mocking.

use std::sync::Arc;

use rekindle_db::paths::DataRoot;
use rekindle_lib::commands::auth::{create_identity_core, lock_identity, login_core};
use rekindle_lib::state::{AppState, SharedState, UserStatus};

/// A data root under `dir`, laid out like the real one, claimed by an
/// in-process host with no node (plan D3.1: login installs the identity in
/// the host), and the state that shares the host's identity slots.
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
    })
    .await
    .unwrap();
    state.host.set(host);
    (root, state)
}

// ── Create Identity ──────────────────────────────────────────────────

#[tokio::test]
async fn create_identity_writes_the_vault_database_and_registry() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let (result, secret) =
        create_identity_core(&root, "test-passphrase", Some("Alice".into()), &state, None)
            .await
            .expect("create_identity_core should succeed");

    assert_eq!(result.display_name, "Alice");
    assert_ne!(secret, [0u8; 32]);
    {
        let identity = state.identity.read();
        let id = identity.as_ref().expect("identity should be set");
        assert_eq!(id.public_key, result.public_key);
        assert_eq!(id.display_name, "Alice");
        assert_eq!(id.status, UserStatus::Online);
    }

    // The identity row is in the identity's own database.
    let pk = result.public_key.clone();
    let name = state
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::identity::login_row(conn, &pk))
        .await
        .unwrap()
        .map(|(name, _)| name);
    assert_eq!(name.as_deref(), Some("Alice"));

    // The unlock is installed: vault, Signal manager, friend authority.
    assert!(state.keystore.lock().is_some());
    assert!(state.signal_manager.read().is_some());
    assert!(state.friend_store.read().is_some());

    // Files and registry.
    assert!(root.identity_vault(&result.public_key).exists());
    assert!(root.identity_db(&result.public_key).exists());
    let listed = rekindle_db::registry::load(&root.registry()).unwrap();
    assert_eq!(
        listed.get(&result.public_key).unwrap().display_name,
        "Alice"
    );
}

#[tokio::test]
async fn create_identity_uses_fallback_display_name() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let (result, _) = create_identity_core(&root, "pass", None, &state, None)
        .await
        .expect("create_identity_core should succeed");

    assert!(
        result.display_name.starts_with("User_"),
        "expected fallback display name, got: {}",
        result.display_name
    );
    assert_eq!(result.display_name.len(), 13); // "User_" + 8 hex chars
}

#[tokio::test]
async fn create_identity_with_empty_display_name_uses_fallback() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let (result, _) = create_identity_core(&root, "pass", Some("   ".into()), &state, None)
        .await
        .unwrap();

    assert!(result.display_name.starts_with("User_"));
}

// ── Login ────────────────────────────────────────────────────────────

#[tokio::test]
async fn login_after_lock_restores_the_same_keypair() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let (created, create_secret) = create_identity_core(
        &root,
        "roundtrip-pass",
        Some("Charlie".into()),
        &state,
        None,
    )
    .await
    .unwrap();

    lock_identity(&state).await;
    assert!(state.identity.read().is_none());
    assert!(state.keystore.lock().is_none());
    assert!(state.db.current().is_err(), "lock closes the database");
    assert!(state.friend_store.read().is_none());

    let (login, login_secret, _) =
        login_core(&root, &created.public_key, "roundtrip-pass", &state, None)
            .await
            .expect("login should succeed with correct passphrase");

    assert_eq!(login.public_key, created.public_key);
    assert_eq!(login.display_name, "Charlie");
    assert_eq!(login_secret, create_secret);
    let identity = state.identity.read();
    let id = identity.as_ref().expect("identity should be restored");
    assert_eq!(id.public_key, created.public_key);
    assert_eq!(id.status, UserStatus::Online);
    assert!(state.keystore.lock().is_some());
}

#[tokio::test]
async fn login_fails_with_wrong_passphrase_and_installs_nothing() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let (created, _) =
        create_identity_core(&root, "correct-pass", Some("Dave".into()), &state, None)
            .await
            .unwrap();
    lock_identity(&state).await;

    let err = login_core(&root, &created.public_key, "wrong-pass", &state, None)
        .await
        .err()
        .expect("login with wrong passphrase should fail");

    assert!(
        err.contains("Wrong passphrase"),
        "expected passphrase error, got: {err}"
    );
    assert!(state.identity.read().is_none());
    assert!(state.keystore.lock().is_none());
    assert!(state.db.current().is_err());
}

#[tokio::test]
async fn login_fails_with_no_identity() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let err = login_core(&root, "nonexistent-key", "any-pass", &state, None)
        .await
        .err()
        .expect("login with no identity should fail");

    assert!(
        err.contains("no identity found"),
        "expected 'no identity found' error, got: {err}"
    );
}

#[tokio::test]
async fn two_identities_coexist_and_unlock_one_at_a_time() {
    let dir = tempfile::TempDir::new().unwrap();
    let (root, state) = test_env(dir.path()).await;

    let (first, _) = create_identity_core(&root, "pass-one", Some("First".into()), &state, None)
        .await
        .unwrap();
    lock_identity(&state).await;
    let (second, _) = create_identity_core(&root, "pass-two", Some("Second".into()), &state, None)
        .await
        .unwrap();
    lock_identity(&state).await;

    let listed = rekindle_db::registry::load(&root.registry()).unwrap();
    assert_eq!(listed.identities.len(), 2);

    let (login, _, _) = login_core(&root, &first.public_key, "pass-one", &state, None)
        .await
        .unwrap();
    assert_eq!(login.display_name, "First");
    lock_identity(&state).await;

    let err = login_core(&root, &second.public_key, "pass-one", &state, None)
        .await
        .err()
        .expect("each identity has its own passphrase");
    assert!(err.contains("Wrong passphrase"), "{err}");
    let (login, _, _) = login_core(&root, &second.public_key, "pass-two", &state, None)
        .await
        .unwrap();
    assert_eq!(login.display_name, "Second");
}
