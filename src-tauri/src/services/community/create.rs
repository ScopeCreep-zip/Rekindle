//! Phase 18.h.2 — thin facade.
//!
//! The full `create_community` pipeline (slot seed + 255 slot pubkeys +
//! 3 SMPL records + genesis governance entries + creator presence +
//! community MEK + CommunityState insert + background services) now
//! lives in `rekindle_governance_runtime::origin::create_community`.
//! This module constructs a `GovernanceAdapter` and delegates.
//!
//! `slot_signing_to_veilid` stays here — it's the Ed25519 → veilid
//! KeyPair conversion needed by segments.rs + join/* and used by the
//! adapter itself via `format_writer_keypair`.

use std::sync::Arc;

use rekindle_secrets::ed25519_dalek::SigningKey;
use tauri::Manager;
use veilid_core::CRYPTO_KIND_VLD0;

use crate::state::AppState;

pub async fn create_community(state: &Arc<AppState>, name: &str) -> Result<String, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<crate::db::DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    let adapter = crate::services::governance_adapter::GovernanceAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    );
    rekindle_governance_runtime::create_community(&adapter, name)
        .await
        .map_err(|e| e.to_string())
}

/// Convert an Ed25519 SigningKey to a Veilid KeyPair for DHT writes.
/// Used by segments.rs (via the adapter's `format_writer_keypair`) +
/// join/helpers.rs (via `try_derive_slot_keypair`).
pub(crate) fn slot_signing_to_veilid(sk: &SigningKey) -> veilid_core::KeyPair {
    let pub_bytes = sk.verifying_key().to_bytes();
    let secret_bytes = sk.to_bytes();
    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let pubkey = veilid_core::PublicKey::new(CRYPTO_KIND_VLD0, bare_pub);
    veilid_core::KeyPair::new_from_parts(pubkey, bare_secret)
}
