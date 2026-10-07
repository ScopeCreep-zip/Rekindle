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
    JoinCommunity(InviteLink),
    AddFriend {
        invite_url: String,
        public_key: String,
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
    /// Add the friend whose identity key has this fingerprint.
    #[serde(rename_all = "camelCase")]
    AddFriend {
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
    FriendAdded,
    Dismissed,
}

/// 64 bits of `blake3(key)`, as four groups of four hex digits.
fn key_fingerprint(key: &str) -> String {
    let hex = rekindle_utils::hash::blake3_hex(key.as_bytes());
    hex.as_bytes()[..16]
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

fn new_request_id() -> String {
    hex::encode(rekindle_utils::random::id_bytes_16())
}

impl PendingDeepLink {
    fn request(&self) -> DeepLinkRequest {
        let request_id = self.request_id.clone();
        match &self.action {
            PendingAction::JoinCommunity(link) => DeepLinkRequest::JoinCommunity {
                request_id,
                key_fingerprint: key_fingerprint(link.governance_key.as_str()),
            },
            PendingAction::AddFriend { public_key, .. } => DeepLinkRequest::AddFriend {
                request_id,
                key_fingerprint: key_fingerprint(public_key),
            },
            PendingAction::PairingRefused => DeepLinkRequest::PairingRefused { request_id },
        }
    }
}

/// Validate a friend invite locally: decode, signature, recency.
fn friend_invite_key(invite_url: &str) -> Result<String, String> {
    let blob = rekindle_codec::message::decode_invite_url(invite_url)?;
    rekindle_codec::message::verify_invite_blob(&blob)?;
    rekindle_codec::message::check_invite_recency(
        &blob,
        rekindle_utils::timestamp_ms(),
        crate::services::friend_runtime::MAX_INVITE_AGE_SECS,
    )?;
    Ok(blob.public_key)
}

/// Classify and validate an OS deep link, hold it for consent, and tell the
/// buddy list when someone is logged in to answer it.
pub fn handle_deep_link_url(app: &AppHandle, url: &str) {
    let Some(state) = app.try_state::<SharedState>() else {
        return;
    };
    let action = match DeepLink::parse(url) {
        Ok(DeepLink::CommunityInvite(link)) => PendingAction::JoinCommunity(link),
        Ok(DeepLink::FriendInvite(invite_url)) => match friend_invite_key(&invite_url) {
            Ok(public_key) => PendingAction::AddFriend {
                invite_url,
                public_key,
            },
            Err(e) => {
                tracing::warn!(error = %e, "deep link: friend invite rejected");
                return;
            }
        },
        Ok(DeepLink::Pairing) => PendingAction::PairingRefused,
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
        crate::event_dispatch::emit_journaled(
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
    app: AppHandle,
    state: &Arc<AppState>,
    pool: &rekindle_db::Db,
    keystore: &crate::keystore::KeystoreHandle,
    request_id: &str,
) -> Result<DeepLinkOutcome, String> {
    match take_matching(state, request_id)? {
        PendingAction::JoinCommunity(link) => {
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
        PendingAction::AddFriend { invite_url, .. } => {
            crate::services::friend_runtime::add_friend_from_invite_inner(
                Arc::clone(state),
                pool.clone(),
                app,
                invite_url,
            )
            .await?;
            Ok(DeepLinkOutcome::FriendAdded)
        }
        PendingAction::PairingRefused => Ok(DeepLinkOutcome::Dismissed),
    }
}

/// Drop the pending deep link the user declined.
pub fn dismiss(state: &AppState, request_id: &str) -> Result<(), String> {
    take_matching(state, request_id).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_four_groups_of_four_hex() {
        let fp = key_fingerprint("VLD0:abc");
        assert_eq!(fp.len(), 19);
        assert!(fp
            .split(' ')
            .all(|g| g.len() == 4 && g.bytes().all(|b| b.is_ascii_hexdigit())));
        assert_ne!(fp, key_fingerprint("VLD0:abd"));
    }
}
