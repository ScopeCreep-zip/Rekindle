//! Integration tests for the complete authentication flow.
//!
//! Tests the real Stronghold keystore, real `SQLite` database, and real Ed25519
//! cryptographic operations in temp directories — no mocking.

use std::sync::Arc;

use rekindle_lib::commands::auth::{create_identity_core, login_core};
use rekindle_lib::db::{self, DbPool};
use rekindle_lib::keystore::{self, KeystoreHandle};
use rekindle_lib::state::{AppState, SharedState, UserStatus};

/// Create fresh test state with an in-memory `SQLite` database.
fn test_state() -> (SharedState, DbPool, KeystoreHandle) {
    let state: SharedState = Arc::new(AppState::default());
    let pool = db::create_pool(":memory:").expect("in-memory SQLite").pool;
    let keystore_handle = keystore::new_handle();
    (state, pool, keystore_handle)
}

// ── Create Identity ──────────────────────────────────────────────────

#[tokio::test]
async fn create_identity_persists_to_db_and_stronghold() {
    let dir = tempfile::TempDir::new().unwrap();
    let (state, pool, ks_handle) = test_state();

    let (result, secret) = create_identity_core(
        dir.path(),
        "test-passphrase",
        Some("Alice".into()),
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .expect("create_identity_core should succeed");

    // Result has correct display name
    assert_eq!(result.display_name, "Alice");
    assert!(!result.public_key.is_empty());

    // Secret key is non-zero
    assert_ne!(secret, [0u8; 32]);

    // Identity stored in AppState
    {
        let identity = state.identity.read();
        let id = identity.as_ref().expect("identity should be set");
        assert_eq!(id.public_key, result.public_key);
        assert_eq!(id.display_name, "Alice");
        assert_eq!(id.status, UserStatus::Online);
    }

    // Identity persisted in SQLite
    let pk = result.public_key.clone();
    let row = pool
        .call(move |conn| {
            conn.query_row(
                "SELECT public_key, display_name FROM identity WHERE public_key = ?",
                rusqlite::params![pk],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(row.0, result.public_key);
    assert_eq!(row.1, "Alice");

    // Keystore handle is populated (session unlocked)
    assert!(ks_handle.lock().is_some());
}

#[tokio::test]
async fn create_identity_uses_fallback_display_name() {
    let dir = tempfile::TempDir::new().unwrap();
    let (state, pool, ks_handle) = test_state();

    let (result, _) = create_identity_core(
        dir.path(),
        "pass",
        None, // No display name
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .expect("create_identity_core should succeed");

    // Fallback: "User_" + first 8 chars of public key
    assert!(
        result.display_name.starts_with("User_"),
        "expected fallback display name, got: {}",
        result.display_name
    );
    assert_eq!(result.display_name.len(), 13); // "User_" + 8 hex chars
}

// ── Login (Success) ──────────────────────────────────────────────────

#[tokio::test]
async fn login_succeeds_with_correct_passphrase() {
    let dir = tempfile::TempDir::new().unwrap();
    let (state, pool, ks_handle) = test_state();

    // First create an identity
    let (create_result, _) = create_identity_core(
        dir.path(),
        "my-secret",
        Some("Bob".into()),
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .unwrap();

    // Clear in-memory state to simulate app restart
    *state.identity.write() = None;
    *ks_handle.lock() = None;

    // Login with correct passphrase
    let (login_result, secret_key, _dht_cols) = login_core(
        dir.path(),
        &create_result.public_key,
        "my-secret",
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .expect("login should succeed with correct passphrase");

    // Same public key and display name restored
    assert_eq!(login_result.public_key, create_result.public_key);
    assert_eq!(login_result.display_name, "Bob");

    // Secret key is non-zero (real key loaded)
    assert_ne!(secret_key, [0u8; 32]);

    // Identity restored in AppState
    let identity = state.identity.read();
    let id = identity.as_ref().expect("identity should be restored");
    assert_eq!(id.public_key, create_result.public_key);
    assert_eq!(id.status, UserStatus::Online);

    // Keystore unlocked for the session
    assert!(ks_handle.lock().is_some());
}

#[tokio::test]
async fn login_restores_same_keypair() {
    let dir = tempfile::TempDir::new().unwrap();
    let (state, pool, ks_handle) = test_state();

    // Create identity and capture secret
    let (create_result, create_secret) = create_identity_core(
        dir.path(),
        "roundtrip-pass",
        Some("Charlie".into()),
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .unwrap();

    // Simulate restart
    *state.identity.write() = None;
    *ks_handle.lock() = None;

    // Login and get secret
    let (login_result, login_secret, _dht_cols) = login_core(
        dir.path(),
        &create_result.public_key,
        "roundtrip-pass",
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .unwrap();

    // Exact same keypair restored
    assert_eq!(login_result.public_key, create_result.public_key);
    assert_eq!(login_secret, create_secret);
}

// ── Login (Failures) ─────────────────────────────────────────────────

#[tokio::test]
async fn login_fails_with_wrong_passphrase() {
    let dir = tempfile::TempDir::new().unwrap();
    let (state, pool, ks_handle) = test_state();

    // Create identity
    let (create_result, _) = create_identity_core(
        dir.path(),
        "correct-pass",
        Some("Dave".into()),
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .unwrap();

    // Simulate restart
    *state.identity.write() = None;
    *ks_handle.lock() = None;

    // Login with wrong passphrase
    let err = login_core(
        dir.path(),
        &create_result.public_key,
        "wrong-pass",
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .expect_err("login with wrong passphrase should fail");

    assert!(
        err.contains("Wrong passphrase") || err.contains("unable to unlock"),
        "expected passphrase error, got: {err}"
    );

    // State should NOT be modified on failure
    assert!(state.identity.read().is_none());
}

#[tokio::test]
async fn login_fails_with_no_identity() {
    let dir = tempfile::TempDir::new().unwrap();
    let (state, pool, ks_handle) = test_state();

    // No identity created — empty database
    let err = login_core(
        dir.path(),
        "nonexistent-key",
        "any-pass",
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .expect_err("login with no identity should fail");

    assert!(
        err.contains("no identity found"),
        "expected 'no identity found' error, got: {err}"
    );
}

// ── Edge Cases ───────────────────────────────────────────────────────

#[tokio::test]
async fn create_identity_with_empty_display_name_uses_fallback() {
    let dir = tempfile::TempDir::new().unwrap();
    let (state, pool, ks_handle) = test_state();

    let (result, _) = create_identity_core(
        dir.path(),
        "pass",
        Some("   ".into()), // Whitespace-only display name
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .unwrap();

    assert!(result.display_name.starts_with("User_"));
}

#[tokio::test]
async fn multiple_create_login_cycles_work() {
    let dir = tempfile::TempDir::new().unwrap();

    // Cycle 1: Create
    let (state, pool, ks_handle) = test_state();
    let (result1, _) = create_identity_core(
        dir.path(),
        "pass-one",
        Some("First".into()),
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .unwrap();

    // Cycle 1: Logout (clear state)
    *state.identity.write() = None;
    *ks_handle.lock() = None;

    // Cycle 1: Login back
    let (login1, _, _dht_cols) = login_core(
        dir.path(),
        &result1.public_key,
        "pass-one",
        &state,
        &pool,
        &ks_handle,
    )
    .await
    .unwrap();

    assert_eq!(login1.public_key, result1.public_key);
    assert_eq!(login1.display_name, "First");
}
