//! Rekindle extension (plan E7.14 S5): a key pressed as native input, so the
//! engine performs its default action (Tab moves focus) and the page sees a
//! trusted event. The W3C key actions dispatch synthetic `KeyboardEvent`s,
//! which a browser never acts on. `webdriver-test` builds only, like the
//! rest of this server.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use tauri::Runtime;

use crate::server::response::{WebDriverResponse, WebDriverResult};
use crate::server::AppState;

/// `{ "key": "Tab", "shift": false }`: a GDK key name, and whether Shift is held.
#[derive(Debug, Deserialize)]
pub struct NativeKeyRequest {
    key: String,
    #[serde(default)]
    shift: bool,
}

/// POST `/session/{session_id}/rekindle/key` - press and release a key as native input
pub async fn send<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(session_id): Path<String>,
    Json(request): Json<NativeKeyRequest>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    executor
        .send_native_key(&request.key, request.shift)
        .await?;
    Ok(WebDriverResponse::null())
}
