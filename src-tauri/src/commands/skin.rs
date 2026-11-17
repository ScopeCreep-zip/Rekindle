//! The skin a window applies (plan E7.7.2 "Apply"): a Host call. The
//! windows re-apply on `SystemEvent::SkinChanged`.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::skin_library::SkinApplied;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// What this window applies: `id`, or with `None` the skin this device
/// shows now (`IpcRequest::SkinGet`).
#[tauri::command]
pub async fn get_skin(
    id: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<SkinApplied>, CommandError> {
    host_call::call(&state, IpcRequest::SkinGet { id }).await
}
