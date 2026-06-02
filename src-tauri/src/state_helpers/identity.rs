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
