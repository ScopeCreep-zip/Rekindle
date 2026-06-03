//! Current-identity accessors.

use std::sync::Arc;

use crate::state::{AppState, IdentityState, UserStatus};

/// Current identity's public key, or error `"not logged in"`.
pub fn current_owner_key(state: &Arc<AppState>) -> Result<String, String> {
    state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .ok_or_else(|| "not logged in".to_string())
}

/// Current identity's public key, or empty string (for non-critical paths).
pub fn owner_key_or_default(state: &Arc<AppState>) -> String {
    state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default()
}

/// Architecture §26 W26 — pull the credentials needed to author a
/// signed DHT subkey write for a community: the pseudonym public (the
/// 32-byte verifying key surfaced on the wire as `author_pseudonym`)
/// and the matching pseudonym signing key derived deterministically
/// from `(identity_secret, community_id)` via
/// `rekindle_secrets::derive::derive_community_pseudonym`.
///
/// Errors when the user isn't logged in (no identity secret) or when
/// the local `CommunityState` lacks the cached pseudonym (which only
/// happens before the join flow finishes priming state).
pub fn pseudonym_credentials(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<(rekindle_types::id::PseudonymKey, ed25519_dalek::SigningKey), String> {
    let secret = {
        let guard = state.identity_secret.lock();
        *guard.as_ref().ok_or("identity secret not available")?
    };
    let signing_key = rekindle_secrets::derive::derive_community_pseudonym(&secret, community_id);
    let pseudo_bytes = signing_key.verifying_key().to_bytes();
    let pseudonym = rekindle_types::id::PseudonymKey(pseudo_bytes);
    let _ = state
        .communities
        .read()
        .get(community_id)
        .ok_or_else(|| "community not found".to_string())?;
    Ok((pseudonym, signing_key))
}

/// Voice self-identity hex — the key we present as *ourselves* on the
/// voice wire. For community voice this is the per-community pseudonym
/// (so `sender_key` matches the pseudonym signing key set on the
/// transport and remote peers can verify our Ed25519 signature); for
/// 1:1 calls it's the owner key (whose verifying key is the identity
/// secret's signing key). Single source of truth for the outbound
/// packet `sender_key`, the receive/MCU self-skip key, the send-loop
/// identity, and the `LocalJoined` roster entry. Empty string when no
/// identity is loaded.
pub fn voice_self_identity(state: &Arc<AppState>, community_id: Option<&str>) -> String {
    match community_id {
        Some(cid) => pseudonym_credentials(state, cid).map_or_else(
            |_| owner_key_or_default(state),
            |(pseudo, _)| hex::encode(pseudo.0),
        ),
        None => owner_key_or_default(state),
    }
}

/// Clone the full identity state, or error `"not logged in"`.
pub fn current_identity(state: &Arc<AppState>) -> Result<IdentityState, String> {
    state
        .identity
        .read()
        .clone()
        .ok_or_else(|| "not logged in".to_string())
}

/// Current identity's display name, or empty string.
pub fn identity_display_name(state: &Arc<AppState>) -> String {
    state
        .identity
        .read()
        .as_ref()
        .map(|id| id.display_name.clone())
        .unwrap_or_default()
}

/// Current identity's status.
pub fn identity_status(state: &Arc<AppState>) -> Option<UserStatus> {
    state.identity.read().as_ref().map(|id| id.status)
}
