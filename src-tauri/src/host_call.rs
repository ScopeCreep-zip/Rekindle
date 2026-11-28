//! The GUI's Host calls (plan E1.1, ADR 0020): a ported command sends its
//! `IpcRequest` through `HostApi` as the shell and answers the Host's
//! response as `Done<T>` or `CommandError`, the shapes every frontend
//! renders. F1 swaps the in-process Host for `DaemonClient` behind the same
//! trait (`HostSlot::api`).

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::Caller;
use rekindle_types::dto::{CommandError, Done};
use serde::de::DeserializeOwned;

use crate::state::AppState;

/// The `window` of a call whose caller does not name its window (a Tauri
/// command is not handed its webview).
pub const UNNAMED_WINDOW: &str = "command";

/// Send `request` to the Host as the window, waiting for the Host to start.
///
/// # Errors
/// The Host refused or failed the request, or holds it for review.
pub async fn call<T: DeserializeOwned>(
    state: &AppState,
    request: IpcRequest,
) -> Result<Done<T>, CommandError> {
    call_from(state, UNNAMED_WINDOW, request).await
}

/// [`call`] for a caller that knows its window's label.
///
/// # Errors
/// The Host refused or failed the request, or holds it for review.
pub async fn call_from<T: DeserializeOwned>(
    state: &AppState,
    window: &str,
    request: IpcRequest,
) -> Result<Done<T>, CommandError> {
    let started = Instant::now();
    let api = state.host.api().await;
    let traced = Traced::new(&request, window, started);
    let response = api.call(Caller::shell(), request).await;
    traced.finish(&response, started.elapsed());
    answer(response)
}

/// One request's debug line (plan T2): the request's variant name only,
/// the caller's window label, when it started, how long it took (waiting
/// for the Host to start included) and the outcome kind. Never a field of
/// the request or the answer: the threat model's no-telemetry rule logs no
/// payload, key, name or content.
/// `RUST_LOG=rekindle=info,rekindle_lib::host_call=debug` enables it alone.
struct Traced<'a> {
    command: &'static str,
    window: &'a str,
    started_ms: u128,
}

impl<'a> Traced<'a> {
    fn new(request: &IpcRequest, window: &'a str, started: Instant) -> Self {
        static FIRST: OnceLock<Instant> = OnceLock::new();
        let first = *FIRST.get_or_init(|| started);
        Self {
            command: request.name(),
            window,
            started_ms: started.saturating_duration_since(first).as_millis(),
        }
    }

    fn finish(&self, response: &IpcResponse, took: Duration) {
        let (outcome, code) = match response {
            IpcResponse::Ok { .. } => ("ok", 0),
            IpcResponse::IntentRequired(_) => ("intent-required", 0),
            IpcResponse::Error { code, .. } => ("error", *code),
        };
        tracing::debug!(
            command = self.command,
            window = self.window,
            started_ms = self.started_ms,
            took_ms = took.as_millis(),
            outcome,
            code,
            "host call"
        );
    }
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

#[cfg(test)]
#[path = "host_call_tests.rs"]
mod tests;
