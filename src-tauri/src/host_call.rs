//! The GUI's Host calls (plan E1.1, ADR 0020): a ported command sends its
//! `IpcRequest` through `HostApi` as the shell and answers the Host's
//! response as `Done<T>` or `CommandError`, the shapes every frontend
//! renders. F1 swaps the in-process Host for `DaemonClient` behind the same
//! trait (`HostSlot::api`).

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::Caller;
use rekindle_types::dto::{CommandError, Done};
use serde::de::DeserializeOwned;

use crate::state::AppState;

/// Send `request` to the Host as the window, waiting for the Host to start.
///
/// # Errors
/// The Host refused or failed the request, or holds it for review.
pub async fn call<T: DeserializeOwned>(
    state: &AppState,
    request: IpcRequest,
) -> Result<Done<T>, CommandError> {
    let api = state.host.api().await;
    answer(api.call(Caller::shell(), request).await)
}

/// The Host's `response` as a command's result.
///
/// # Errors
/// The response is an error or a review, or its value is not a `T`.
pub fn answer<T: DeserializeOwned>(response: IpcResponse) -> Result<Done<T>, CommandError> {
    match response {
        IpcResponse::Ok { data, undo, .. } => serde_json::from_value(data)
            .map(|value| Done { value, undo })
            .map_err(|e| CommandError::internal(format!("the host's answer: {e}"))),
        IpcResponse::IntentRequired(review) => Err(CommandError::IntentRequired(review)),
        IpcResponse::Error {
            code,
            message,
            remediation,
        } => Err(CommandError::Failed {
            code,
            message,
            remediation,
        }),
    }
}
