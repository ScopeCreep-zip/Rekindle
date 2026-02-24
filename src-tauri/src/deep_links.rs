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
///   `rekindle://community/{community_id}/{invite_code}`
pub fn handle_deep_link_url(app: &AppHandle, url: &str) {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("rekindle://community/") {
        let rest = rest.trim_end_matches('/');
        // Expect: {community_id}/{invite_code}
        if let Some((community_id, invite_code)) = rest.split_once('/') {
            if !community_id.is_empty() && !invite_code.is_empty() {
                let action = DeepLinkAction {
                    action: "joinCommunity".into(),
                    community_id: community_id.to_string(),
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
                            community = %community_id,
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
