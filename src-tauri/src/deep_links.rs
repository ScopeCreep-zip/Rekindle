//! OS deep links (`rekindle://`), held for explicit user consent.
//!
//! A deep link can arrive from any web page or app, so nothing it names is
//! acted on until the user confirms it in the buddy list:
//!
//! 1. [`handle_deep_link_url`] classifies the URL (`rekindle_types::invite::DeepLink`)
//!    and validates it locally: no DHT or network access.
//! 2. The parsed link stays in `AppState::pending_deep_link`, keyed by a
//!    random request id. The webview sees only a [`DeepLinkRequest`]: the
//!    request id and a 64-bit fingerprint of the key it names, never the
//!    invite code, secrets key or friend-invite blob.
//! 3. The buddy list pulls the request ([`pending_request`]) on mount and
//!    on the journaled `deep-link-action` event, and shows a consent
//!    dialog. Only [`confirm`] with the matching id acts on it.
//!
//! A new deep link replaces an unanswered one. Logout clears it, so a link
//! received before user A logged out is never offered to user B.
//!
//! A friend invite is not held here (plan E2.1): it is the Host's
//! `FriendAddFromInvite`, which the gate holds for review, and the buddy
//! list shows the review as its fingerprint card (`IntentPending`). Before
//! unlock the raw link waits in `AppState::held_friend_invite` (the newest
//! wins) until the unlock submits it.

use std::sync::Arc;

use rekindle_types::invite::{DeepLink, InviteLink};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::{AppState, SharedState};

/// A deep link awaiting the user's decision.
pub struct PendingDeepLink {
    request_id: String,
    action: PendingAction,
}

enum PendingAction {
    JoinCommunity {
        link: InviteLink,
        fingerprint: String,
    },
    PairingRefused,
}

/// What the consent dialog shows. Secrets never cross into the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DeepLinkRequest {
    /// Join the community whose governance key has this fingerprint.
    #[serde(rename_all = "camelCase")]
    JoinCommunity {
        request_id: String,
        key_fingerprint: String,
    },
    /// A pairing link arrived from the OS and was refused; the dialog tells
    /// the user to paste pairing codes in Settings → Devices.
    #[serde(rename_all = "camelCase")]
    PairingRefused { request_id: String },
}

/// What a confirmed deep link did, so the buddy list can refresh.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DeepLinkOutcome {
    #[serde(rename_all = "camelCase")]
    JoinedCommunity {
        community_id: String,
    },
    Dismissed,
}

fn new_request_id() -> String {
    hex::encode(rekindle_utils::random::id_bytes_16())
}

impl PendingDeepLink {
    fn request(&self) -> DeepLinkRequest {
        let request_id = self.request_id.clone();
        match &self.action {
            PendingAction::JoinCommunity { fingerprint, .. } => DeepLinkRequest::JoinCommunity {
                request_id,
                key_fingerprint: fingerprint.clone(),
            },
            PendingAction::PairingRefused => DeepLinkRequest::PairingRefused { request_id },
        }
    }
}

/// A friend invite from the OS: reviewed by the Host now while an identity
/// is unlocked, else held until the unlock submits it.
fn friend_invite(state: &Arc<AppState>, invite: String) {
    if state.identity.read().is_some() {
        let task_state = Arc::clone(state);
        crate::state_helpers::spawn_in_login(state, "friend invite review", async move {
            review_friend_invite(&task_state, invite).await;
        });
    } else {
        *state.held_friend_invite.lock() = Some(invite);
        tracing::info!("friend invite link held until login");
    }
}

/// Ask the Host to add the friend `invite` names, as the window (only a GUI
/// window commits it: OS input reached the GUI, not a bus client). The gate
/// holds it for review; the review is published for the buddy list's
/// fingerprint card.
pub async fn review_friend_invite(state: &AppState, invite: String) {
    let api = state.host.api().await;
    let request = rekindle_ipc::protocol::IpcRequest::FriendAddFromInvite { invite };
    match api.call(rekindle_ipc::Caller::shell(), request).await {
        rekindle_ipc::protocol::IpcResponse::IntentRequired(review) => {
            if let Some(host) = state.host.current() {
                host.publish(
                    rekindle_types::subscription_events::SubscriptionEvent::IntentPending(review),
                );
            }
        }
        rekindle_ipc::protocol::IpcResponse::Error { message, .. } => {
            tracing::warn!(error = %message, "deep link: friend invite rejected");
        }
        rekindle_ipc::protocol::IpcResponse::Ok { .. } => {
            tracing::warn!("deep link: a friend invite ran without review");
        }
    }
}

/// Classify and validate an OS deep link, hold it for consent, and tell the
/// buddy list when someone is logged in to answer it.
pub fn handle_deep_link_url(app: &AppHandle, url: &str) {
    let Some(state) = app.try_state::<SharedState>() else {
        return;
    };
    let action = match DeepLink::parse(url) {
        Ok(DeepLink::CommunityInvite(link)) => {
            let Some(fingerprint) =
                rekindle_crypto::identity::record_key_fingerprint(link.governance_key.as_str())
            else {
                tracing::warn!("deep link: community invite key is not a record key");
                return;
            };
            PendingAction::JoinCommunity { link, fingerprint }
        }
        Ok(DeepLink::FriendInvite(invite_url)) => {
            friend_invite(&state, invite_url);
            return;
        }
        Ok(DeepLink::Pairing) => PendingAction::PairingRefused,
        Ok(DeepLink::Add(_)) => {
            tracing::warn!("deep link: friend code not yet submitted (E2.5 S3 part 3)");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "deep link rejected");
            return;
        }
    };
    let pending = PendingDeepLink {
        request_id: new_request_id(),
        action,
    };
    let request = pending.request();
    *state.pending_deep_link.lock() = Some(pending);
    if state.identity.read().is_some() {
        crate::event_dispatch::emit_from_state(
            &state,
            crate::event_dispatch::WebviewEvent::DeepLink(request),
        );
    } else {
        tracing::info!("deep link held until login");
    }
}

/// The deep link awaiting consent, if any.
pub fn pending_request(state: &AppState) -> Option<DeepLinkRequest> {
    state
        .pending_deep_link
        .lock()
        .as_ref()
        .map(PendingDeepLink::request)
}

/// Remove and return the pending link if `request_id` names it.
fn take_matching(state: &AppState, request_id: &str) -> Result<PendingAction, String> {
    let mut slot = state.pending_deep_link.lock();
    if slot.as_ref().is_some_and(|p| p.request_id == request_id) {
        if let Some(pending) = slot.take() {
            return Ok(pending.action);
        }
    }
    Err("this link request is no longer pending".to_owned())
}

/// Act on the pending deep link the user confirmed.
pub async fn confirm(
    state: &Arc<AppState>,
    pool: &rekindle_db::Db,
    keystore: &rekindle_vault::VaultHandle,
    request_id: &str,
) -> Result<DeepLinkOutcome, String> {
    match take_matching(state, request_id)? {
        PendingAction::JoinCommunity { link, .. } => {
            let _g = rekindle_lifecycle::TransportGuard::write(&state.lifecycle)
                .map_err(|e| e.to_string())?;
            crate::services::community_lifecycle_runtime::join_community_inner(
                state, pool, keystore, &link,
            )
            .await?;
            Ok(DeepLinkOutcome::JoinedCommunity {
                community_id: link.governance_key.into_string(),
            })
        }
        PendingAction::PairingRefused => Ok(DeepLinkOutcome::Dismissed),
    }
}

/// Drop the pending deep link the user declined.
pub fn dismiss(state: &AppState, request_id: &str) -> Result<(), String> {
    take_matching(state, request_id).map(drop)
}
