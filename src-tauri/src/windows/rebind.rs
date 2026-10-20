//! Windows bound to a friend's key follow their identity rotation (plan
//! audit R-5): on `FriendEvent::KeyChanged` the chat and profile windows
//! opened for the old key are closed and opened again for the new one, so
//! their label, route and event audience name the key the Host now uses.

use rekindle_types::key_format;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};
use tauri::{AppHandle, Manager};

use crate::state::SharedState;
use crate::window_labels as labels;

/// Rebind `old`'s chat and profile windows to `new`. Off the event loop:
/// a reopened window resolves its title from the database.
fn peer_key_changed(app: &AppHandle, state: &SharedState, old: &str, new: &str) {
    let (Ok(old), Ok(new)) = (
        key_format::public_key_hex(old),
        key_format::public_key_hex(new),
    ) else {
        tracing::warn!("key change names a malformed key; no window rebound");
        return;
    };
    let close = |label: String| {
        app.get_webview_window(&label)
            .map(|window| {
                if let Err(e) = window.close() {
                    tracing::warn!(error = %e, label, "closing a window for a rotated key failed");
                }
            })
            .is_some()
    };
    let chat = close(labels::chat_label(&old));
    let profile = close(labels::profile_label(&old));
    if !chat && !profile {
        return;
    }
    let scope = crate::state_helpers::login_scope_or_closed(state);
    let (app, state) = (app.clone(), state.clone());
    scope.spawn_or_drop("rebind rotated key's windows", async move {
        let Ok(db) = state.db.current() else {
            return;
        };
        let peer = new.as_str().to_owned();
        if chat {
            let target = super::ChatTarget::Peer(peer.clone());
            if let Err(e) = super::open_chat_window(&app, &state, &db, &target).await {
                tracing::warn!(error = %e, "reopening the chat for a rotated key failed");
            }
        }
        if profile {
            if let Err(e) = super::open_profile_window(&app, &state, &db, &peer).await {
                tracing::warn!(error = %e, "reopening the profile for a rotated key failed");
            }
        }
    });
}

/// The shell's own reactions to a Host event, before it reaches the
/// windows: a review the backend began is answered on the buddy list, so
/// it is brought forward (never recreated, which would drop the review);
/// a friend's rotated identity rebinds their windows (plan audit R-5).
pub fn follow_event(app: &AppHandle, state: &SharedState, event: &SubscriptionEvent) {
    match event {
        SubscriptionEvent::IntentPending(_) => super::surface_buddy_list(app),
        SubscriptionEvent::Friend(FriendEvent::KeyChanged { peer_key, new_key }) => {
            peer_key_changed(app, state, peer_key, new_key);
        }
        _ => {}
    }
}
