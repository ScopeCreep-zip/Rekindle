use tauri::{AppHandle, Manager};

use crate::state::SharedState;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkAction {
    pub action: String,
    pub community_id: String,
    pub secrets_record_key: String,
    pub invite_code: String,
}

/// Parse a `rekindle://` URL and emit a frontend event.
///
/// If the user is not yet authenticated, the action is stored in
/// `AppState::pending_deep_link` and replayed after login.
///
/// Supported formats:
///   `rekindle://invite/{governance_key}/{secrets_record_key}/{invite_code}`
pub fn handle_deep_link_url(app: &AppHandle, url: &str) {
    // Parsing lives in `rekindle_types::invite::InviteLink` so every
    // frontend accepts the same link — this file used to be the only
    // place that understood the format, which left the CLI unable to act
    // on a `rekindle://invite/...` URL its own IPC accepts.
    let Some(link) = rekindle_types::invite::InviteLink::parse(url) else {
        return;
    };
    let community_id = link.governance_key.clone();
    let action = DeepLinkAction {
        action: "joinCommunity".into(),
        community_id: link.governance_key,
        secrets_record_key: link.secrets_record_key,
        invite_code: link.invite_code,
    };

    // Emit only when authenticated; otherwise queue for replay on login.
    let is_authed = app
        .try_state::<SharedState>()
        .is_some_and(|state| state.identity.read().is_some());
    if is_authed {
        crate::event_dispatch::emit_live(app, "deep-link-action", &action);
    } else if let Some(state) = app.try_state::<SharedState>() {
        *state.pending_deep_link.lock() = Some(action);
        tracing::info!(
            community = %community_id,
            "deep link queued — will replay after login"
        );
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
            crate::event_dispatch::emit_live(app, "deep-link-action", &action);
        }
    }
}
