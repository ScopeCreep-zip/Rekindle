use tauri::{AppHandle, Emitter, Manager};

use crate::state::SharedState;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkAction {
    pub action: String,
    pub community_id: String,
    pub invite_code: String,
}

/// Parse a `rekindle://` URL and emit a frontend event.
///
/// If the user is not yet authenticated, the action is stored in
/// `AppState::pending_deep_link` and replayed after login.
///
/// Supported formats:
///   `rekindle://invite/{governance_key}/{invite_code}`
pub fn handle_deep_link_url(app: &AppHandle, url: &str) {
    let url = url.trim();
    // Parse: rekindle://invite/{governance_key}/{invite_code}
    let rest = url
        .strip_prefix("rekindle://invite/")
        .or_else(|| url.strip_prefix("rekindle://community/"));
    if let Some(rest) = rest {
        let rest = rest.trim_end_matches('/');
        // Expect: {governance_key}/{invite_code}
        if let Some((governance_key, invite_code)) = rest.split_once('/') {
            if !governance_key.is_empty() && !invite_code.is_empty() {
                let action = DeepLinkAction {
                    action: "joinCommunity".into(),
                    community_id: governance_key.to_string(),
                    invite_code: invite_code.to_string(),
                };

                // Check if user is authenticated before emitting
                let is_authed = app
                    .try_state::<SharedState>()
                    .is_some_and(|state| state.identity.read().is_some());

                if is_authed {
                    let _ = app.emit("deep-link-action", action);
                } else {
                    // Queue for replay after login
                    if let Some(state) = app.try_state::<SharedState>() {
                        *state.pending_deep_link.lock() = Some(action);
                        tracing::info!(
                            community = %governance_key,
                            "deep link queued — will replay after login"
                        );
                    }
                }
            }
        }
    }
}

/// Emit any pending deep link action that was received before authentication.
///
/// Called after successful login to replay the queued action.
pub fn emit_pending_deep_link(app: &AppHandle) {
    if let Some(state) = app.try_state::<SharedState>() {
        let action = state.pending_deep_link.lock().take();
        if let Some(action) = action {
            tracing::info!(
                community = %action.community_id,
                "replaying queued deep link after login"
            );
            let _ = app.emit("deep-link-action", action);
        }
    }
}
