use tauri::State;

use crate::db::DbPool;
use crate::keystore::KeystoreHandle;
use crate::services;
pub use crate::services::auth_cores::LoginResult;
pub use crate::services::auth_runtime::IdentitySummary;
use crate::state::SharedState;

/// Core identity creation logic, separated from `AppHandle` for testability.
///
/// Generates keypair, stores in Stronghold + `SQLite`, sets `AppState`.
/// Returns `(LoginResult, secret_key_bytes)` so the caller can decide
/// whether to spawn background services.
///
/// Multiple identities can coexist — each gets its own Stronghold file
/// and `owner_key`-scoped rows. Only one is active at a time.
pub use services::auth_cores::{create_identity_core, login_core, IdentityDhtColumns};

/// Create a new identity (first-time setup).
///
/// Generates an Ed25519 keypair, stores the private key in Stronghold
/// (encrypted with passphrase via `Argon2id`), and persists public identity to `SQLite`.
#[tauri::command]
pub async fn create_identity(
    passphrase: String,
    display_name: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<LoginResult, String> {
    services::auth_runtime::create_identity_inner(
        passphrase,
        display_name,
        app,
        state.inner().clone(),
        pool.inner().clone(),
        keystore_handle.inner().clone(),
    )
    .await
}

/// Unlock existing identity with passphrase.
///
/// Opens the per-identity Stronghold snapshot with the passphrase, loads the
/// Ed25519 private key, verifies it matches the `SQLite` public key, and
/// restores friends + communities.
#[tauri::command]
pub async fn login(
    public_key: String,
    passphrase: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<LoginResult, String> {
    services::auth_runtime::login_inner(
        public_key,
        passphrase,
        app,
        state.inner().clone(),
        pool.inner().clone(),
        keystore_handle.inner().clone(),
    )
    .await
}

/// Get the current identity state.
///
/// Used by newly opened windows to hydrate their local auth state
/// from the shared Rust backend (each webview has isolated JS context).
#[tauri::command]
pub async fn get_identity(state: State<'_, SharedState>) -> Result<Option<LoginResult>, String> {
    let identity = state.identity.read();
    Ok(identity.as_ref().map(|id| LoginResult {
        public_key: id.public_key.clone(),
        display_name: id.display_name.clone(),
    }))
}

/// Log out: save and lock Stronghold, clean up user state, keep node alive.
#[tauri::command]
pub async fn logout(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    services::auth_runtime::logout_inner(
        app,
        state.inner().clone(),
        keystore_handle.inner().clone(),
    )
    .await
}

/// List all persisted identities (for the account picker).
///
/// Returns summaries of every identity in `SQLite`, ordered by creation date.
/// No authentication needed — this is called by the login window on mount.
#[tauri::command]
pub async fn list_identities(pool: State<'_, DbPool>) -> Result<Vec<IdentitySummary>, String> {
    services::auth_runtime::list_identities_inner(pool.inner()).await
}

/// Delete a specific identity after verifying the passphrase.
///
/// Opens the identity's Stronghold to verify the passphrase, then deletes:
/// - The identity row (CASCADE deletes all scoped data)
/// - The Stronghold snapshot file
///
/// If deleting the currently active identity, performs logout first.
#[tauri::command]
pub async fn delete_identity(
    public_key: String,
    passphrase: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    services::auth_runtime::delete_identity_inner(
        public_key,
        passphrase,
        app,
        state.inner().clone(),
        pool.inner().clone(),
        keystore_handle.inner().clone(),
    )
    .await
}

/// Phase 3b debug diagnostic — inspect the local PQXDH PreKeyBundle.
///
/// Returns the byte lengths of each bundle component so the dev console
/// can confirm ML-KEM-768 keys (1184 B public, 1088 B ciphertext) and
/// classical X3DH keys (32 B X25519, 64 B Ed25519 sig) are wired through
/// the full publish path. Debug-only — gated on `cfg(debug_assertions)`
/// so release builds never surface key sizes.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn pqxdh_bundle_info(
    state: State<'_, SharedState>,
) -> Result<crate::services::auth_runtime::PqxdhBundleInfo, String> {
    services::auth_runtime::pqxdh_bundle_info_inner(state.inner())
}

/// Phase 4 — verify the local audit hash chain end-to-end.
///
/// Returns `{ ok, length, brokenAt }`. On failure, also emits
/// `notification-event::SystemAlert` so the user sees the tamper signal.
#[tauri::command]
pub async fn audit_verify(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<crate::audit_repo::AuditVerifyResult, String> {
    services::auth_runtime::audit_verify_inner(app, state.inner(), pool.inner()).await
}

/// Phase 4 — export audit entries with `cursor > since`. Used by dev tooling
/// and any future "download my audit trail" feature.
#[tauri::command]
pub async fn audit_export(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    since: u64,
) -> Result<Vec<rekindle_audit::AuditEntry>, String> {
    services::auth_runtime::audit_export_inner(state.inner(), pool.inner(), since).await
}

/// Phase 5 — read the current lifecycle state.
///
/// Frontend uses this to disable buttons when the FSM doesn't accept
/// writes (e.g. show "Reconnecting…" badge in `Detached`, gate the send
/// button while `Resuming`). The richer `lifecycle-event` channel
/// pushes transitions live; this command is the cold-start fallback.
#[tauri::command]
pub async fn lifecycle_current(
    state: State<'_, SharedState>,
) -> Result<rekindle_lifecycle::LifecycleState, String> {
    Ok(state.lifecycle.state())
}

/// Phase 7 — fire an immediate friend-inbox scan via the coordinator's
/// direct-trigger channel. No-op (debug-warn) if no coordinator is
/// running yet (e.g., user invokes before login completes).
#[tauri::command]
pub async fn friendship_scan_now(state: State<'_, SharedState>) -> Result<(), String> {
    state.inner().friendship_handle.scan_now().await;
    Ok(())
}

/// Phase 7 — disable the watch tier for `duration_ms`. Direct triggers
/// and the 30-second poll backstop continue to operate; only the
/// Veilid-DHT-ValueChanged → watch_tx path is suppressed. Used by the
/// plan's manual test: "On B, disable the watch for 60s; send a friend
/// request; expect arrival within ~30s via poll tier."
///
/// Available in release builds because the watch tier is a production
/// feature and operators may want to manually verify the poll
/// backstop. No security impact: local-only Tauri IPC, no secrets, no
/// privilege bypass — at worst delays friend-request delivery for the
/// supplied duration on this device only.
#[tauri::command]
pub async fn dev_disable_watch(
    state: State<'_, SharedState>,
    duration_ms: u64,
) -> Result<(), String> {
    state
        .inner()
        .friendship_handle
        .dev_disable_watch(std::time::Duration::from_millis(duration_ms));
    Ok(())
}
