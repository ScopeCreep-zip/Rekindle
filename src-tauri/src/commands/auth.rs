//! The identity commands are Host calls (plan E1.1): the Host runs create,
//! unlock, logout and delete through its one pipeline and intent gate, and
//! answers the DTOs every frontend renders.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::identity::{IdentityShown, IdentitySummary, IdentityView};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Create a new identity and unlock it (`IpcRequest::IdentityCreate`).
/// Without a name it is `User_` and its key's first eight characters.
#[tauri::command]
pub async fn create_identity(
    passphrase: String,
    display_name: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<IdentityView>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::IdentityCreate {
            display_name,
            passphrase,
        },
    )
    .await
}

/// Unlock an identity with its passphrase (`IpcRequest::Unlock`).
#[tauri::command]
pub async fn login(
    public_key: String,
    passphrase: String,
    state: State<'_, SharedState>,
) -> Result<Done<IdentityView>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Unlock {
            identity: public_key,
            passphrase,
        },
    )
    .await
}

/// The unlocked identity, or `None` while locked (`IpcRequest::IdentityShow`):
/// each window hydrates its auth state from it.
#[tauri::command]
pub async fn get_identity(
    state: State<'_, SharedState>,
) -> Result<Done<Option<IdentityShown>>, CommandError> {
    host_call::call(&state, IpcRequest::IdentityShow).await
}

/// Log out: the Host ends the session and locks the identity; the node
/// stays up, and the login window opens (`IpcRequest::Logout`).
#[tauri::command]
pub async fn logout(
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(&state, IpcRequest::Logout).await
}

/// The identities on this device, oldest first, for the account picker
/// (`IpcRequest::IdentityList`; no unlock needed).
#[tauri::command]
pub async fn list_identities(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<IdentitySummary>>, CommandError> {
    host_call::call(&state, IpcRequest::IdentityList).await
}

/// Delete an identity (`IpcRequest::IdentityDelete`). It is Protected: the
/// Host holds it for the identity's passphrase, which the intent dialog
/// asks for, and ends its session first when it is the unlocked one.
#[tauri::command]
pub async fn delete_identity(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::IdentityDelete {
            identity: public_key,
        },
    )
    .await
}

/// This device's prekey record and its upkeep (`IpcRequest::PrekeyStatus`,
/// plan E2.4): the record key, the signed prekey's and last-resort key's
/// ids and ages, the fresh and consumed slots, and the published head's
/// component sizes, as the CLI's `keys prekeys status` shows them. Debug
/// builds only, so release builds never surface key sizes.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn pqxdh_bundle_info(
    state: State<'_, SharedState>,
) -> Result<Done<rekindle_types::dto::identity::PrekeyStatus>, CommandError> {
    host_call::call(&state, IpcRequest::PrekeyStatus).await
}

/// Verify the identity's tamper-evident audit log end to end
/// (`IpcRequest::AuditVerify`); a broken chain also raises the tamper alert.
#[tauri::command]
pub async fn audit_verify(
    state: State<'_, SharedState>,
) -> Result<Done<rekindle_types::dto::audit::AuditVerified>, CommandError> {
    host_call::call(&state, IpcRequest::AuditVerify).await
}

/// The audit log's entries after `since` (`IpcRequest::AuditExport`, a
/// Protected action the intent dialog proves).
#[tauri::command]
pub async fn audit_export(
    state: State<'_, SharedState>,
    since: u64,
) -> Result<Done<Vec<rekindle_types::dto::audit::AuditEntryView>>, CommandError> {
    host_call::call(&state, IpcRequest::AuditExport { since }).await
}

/// The Host's lifecycle state (`IpcRequest::Status`): the cold-start read
/// the login button and the state badges seed from; the `lifecycle-event`
/// channel pushes transitions live.
#[tauri::command]
pub async fn lifecycle_current(
    state: State<'_, SharedState>,
) -> Result<Done<rekindle_lifecycle::LifecycleState>, CommandError> {
    let status: Done<rekindle_types::display::StatusSnapshot> =
        host_call::call(&state, IpcRequest::Status).await?;
    let value = serde_json::from_value(serde_json::Value::String(status.value.state))
        .map_err(|e| CommandError::internal(format!("the host's lifecycle state: {e}")))?;
    Ok(Done {
        value,
        undo: status.undo,
    })
}
