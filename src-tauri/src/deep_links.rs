//! OS deep links (`rekindle://`): parsed by the shell and submitted to the
//! Host, whose gate holds each action for the user's review.
//!
//! A deep link can arrive from any web page or app, so nothing it names
//! runs unreviewed:
//!
//! - a community invite is the Host's `CommunityJoin`, which the gate holds
//!   as a TrustChanging `JoinCommunity` whose subject is the fingerprint of
//!   the invite's record key (plan E3.1 D2);
//! - a friend invite or friend code is the Host's `FriendAddFromInvite` or
//!   `FriendAddFromCode`, held the same way (plan E2.1);
//! - a pairing link is refused: pairing codes are accepted only in
//!   Settings, and the buddy list says so (`WebviewEvent::PairingLinkRefused`).
//!
//! Each review is published as `IntentPending` for the buddy list's intent
//! dialog. A link that arrives before unlock waits in
//! `AppState::held_deep_link` as the raw OS input (the newest wins) until the
//! unlock submits it; logout clears it, so a link that reached user A is
//! never offered to user B.

use std::sync::Arc;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse};
use rekindle_types::invite::DeepLink;
use tauri::{AppHandle, Manager};

use crate::state::{AppState, SharedState};

/// An OS deep link: submitted now while an identity is unlocked, else held
/// as raw input until the unlock submits it.
pub fn handle_deep_link_url(app: &AppHandle, url: &str) {
    let Some(state) = app.try_state::<SharedState>() else {
        return;
    };
    if state.identity.read().is_some() {
        let (task_state, url) = (Arc::clone(&state), url.to_owned());
        crate::state_helpers::spawn_in_login(&state, "deep link review", async move {
            submit(&task_state, &url).await;
        });
    } else {
        *state.held_deep_link.lock() = Some(url.to_owned());
        tracing::info!("deep link held until login");
    }
}

/// Classify `url` and hand what it asks for to the Host as the window
/// (OS input reached the GUI, not a bus client); a pairing link is refused.
pub async fn submit(state: &AppState, url: &str) {
    let request = match DeepLink::parse(url) {
        Ok(DeepLink::CommunityInvite(link)) => {
            IpcRequest::Community(CommunityRequest::CommunityJoin {
                invite: link.to_url(),
            })
        }
        Ok(DeepLink::FriendInvite(invite)) => IpcRequest::FriendAddFromInvite { invite },
        Ok(DeepLink::Add(code)) => IpcRequest::FriendAddFromCode {
            code: code.to_url(),
            message: String::new(),
        },
        Ok(DeepLink::Pairing) => {
            crate::event_dispatch::emit_from_state(
                state,
                crate::event_dispatch::WebviewEvent::PairingLinkRefused,
            );
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "deep link rejected");
            return;
        }
    };
    let api = state.host.api().await;
    match api.call(rekindle_ipc::Caller::shell(), request).await {
        IpcResponse::IntentRequired(review) => {
            if let Some(host) = state.host.current() {
                host.publish(
                    rekindle_types::subscription_events::SubscriptionEvent::IntentPending(review),
                );
            }
        }
        IpcResponse::Error { message, .. } => {
            tracing::warn!(error = %message, "deep link: the Host refused it");
        }
        IpcResponse::Ok { .. } => {
            tracing::warn!("deep link: an action ran without review");
        }
    }
}
