//! The identity commands are Host calls (plan E1.1): the Host runs create,
//! unlock, logout and delete through its one pipeline and intent gate, and
//! answers the DTOs every frontend renders.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::identity::{IdentityShown, IdentitySummary, IdentityView};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::passphrase::Passphrase;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

host_command! {
    state;
    /// Create a new identity and unlock it (`IpcRequest::IdentityCreate`).
    /// Without a name it is `User_` and its key's first eight characters.
    pub async fn create_identity(
        passphrase: Passphrase,
        display_name: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<IdentityView>, CommandError> {
        IpcRequest::IdentityCreate {
            display_name,
            passphrase,
        }
    }
}

host_command! {
    state;
    /// Unlock an identity with its passphrase (`IpcRequest::Unlock`).
    pub async fn login(
        public_key: String,
        passphrase: Passphrase,
        state: State<'_, SharedState>,
    ) -> Result<Done<IdentityView>, CommandError> {
        IpcRequest::Unlock {
            identity: public_key,
            passphrase,
        }
    }
}

host_command! {
    state;
    /// The unlocked identity, or `None` while locked (`IpcRequest::IdentityShow`):
    /// each window hydrates its auth state from it.
    pub async fn get_identity(
        state: State<'_, SharedState>,
    ) -> Result<Done<Option<IdentityShown>>, CommandError> {
        IpcRequest::IdentityShow
    }
}

host_command! {
    state;
    /// Log out: the Host ends the session and locks the identity; the node
    /// stays up, and the login window opens (`IpcRequest::Logout`).
    pub async fn logout(
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Logout
    }
}

host_command! {
    state;
    /// The identities on this device, oldest first, for the account picker
    /// (`IpcRequest::IdentityList`; no unlock needed).
    pub async fn list_identities(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<IdentitySummary>>, CommandError> {
        IpcRequest::IdentityList
    }
}

host_command! {
    state;
    /// Delete an identity (`IpcRequest::IdentityDelete`). It is Protected: the
    /// Host holds it for the identity's passphrase, which the intent dialog
    /// asks for, and ends its session first when it is the unlocked one.
    pub async fn delete_identity(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::IdentityDelete {
            identity: public_key,
        }
    }
}

#[cfg(debug_assertions)]
host_command! {
    state;
    /// This device's prekey record and its upkeep (`IpcRequest::PrekeyStatus`,
    /// plan E2.4): the record key, the signed prekey's and last-resort key's
    /// ids and ages, the fresh and consumed slots, and the published head's
    /// component sizes, as the CLI's `keys prekeys status` shows them. Debug
    /// builds only, so release builds never surface key sizes.
    pub async fn pqxdh_bundle_info(
        state: State<'_, SharedState>,
    ) -> Result<Done<rekindle_types::dto::identity::PrekeyStatus>, CommandError> {
        IpcRequest::PrekeyStatus
    }
}

host_command! {
    state;
    /// Verify the identity's tamper-evident audit log end to end
    /// (`IpcRequest::AuditVerify`); a broken chain also raises the tamper alert.
    pub async fn audit_verify(
        state: State<'_, SharedState>,
    ) -> Result<Done<rekindle_types::dto::audit::AuditVerified>, CommandError> {
        IpcRequest::AuditVerify
    }
}

/// Ask where to save the audit log's entries after `since`, then have the
/// Host write them there (`IpcRequest::AuditExport`, a Protected action
/// the intent dialog proves): the path and entry count, or `None` if the
/// user cancelled the save dialog.
#[tauri::command]
pub async fn audit_export(
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
    since: u64,
) -> Result<Done<Option<rekindle_types::dto::audit::AuditExported>>, CommandError> {
    let Some(path) =
        crate::services::community_files_runtime::save_path(&window, "rekindle-audit-log.json")
            .await?
    else {
        return Ok(Done {
            value: None,
            undo: None,
        });
    };
    let done: Done<rekindle_types::dto::audit::AuditExported> =
        host_call::call(&state, IpcRequest::AuditExport { since, path }).await?;
    Ok(Done {
        value: Some(done.value),
        undo: done.undo,
    })
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
