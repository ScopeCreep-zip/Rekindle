//! The PQXDH bundle diagnostic, debug builds only (→ E2.4, which rewrites
//! it onto the prekey record). The identity lifecycle and the audit log
//! are the Host's (plan E1.1).

use std::sync::Arc;

use serde::Serialize;

use crate::state::AppState;

/// Phase 3b debug diagnostic — inspect the local PQXDH PreKeyBundle.
///
/// Returns the byte lengths of each bundle component so the dev console
/// can confirm ML-KEM-768 keys (1184 B public, 1088 B ciphertext) and
/// classical X3DH keys (32 B X25519, 64 B Ed25519 sig) are wired through
/// the full publish path. Debug-only — gated on `cfg(debug_assertions)`
/// so release builds never surface key sizes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PqxdhBundleInfo {
    pub identity_key_len: usize,
    pub signed_prekey_len: usize,
    pub signed_prekey_signature_len: usize,
    pub one_time_prekey_len: Option<usize>,
    pub registration_id: u32,
    pub pqpk_lr_len: usize,
    pub pqpk_lr_signature_len: usize,
    pub pqpk_ot_len: Option<usize>,
    pub pqpk_ot_signature_len: Option<usize>,
}

pub fn pqxdh_bundle_info_inner(state: &Arc<AppState>) -> Result<PqxdhBundleInfo, String> {
    let handle = state
        .signal_manager
        .read()
        .as_ref()
        .map(std::sync::Arc::clone)
        .ok_or("signal manager not initialized")?;
    let bundle = handle
        .manager
        .current_bundle()
        .map_err(|e| format!("current bundle: {e}"))?;
    Ok(PqxdhBundleInfo {
        identity_key_len: bundle.identity_key.len(),
        signed_prekey_len: bundle.signed_prekey.len(),
        signed_prekey_signature_len: bundle.signed_prekey_signature.len(),
        one_time_prekey_len: bundle.one_time_prekey.as_ref().map(Vec::len),
        registration_id: bundle.registration_id,
        pqpk_lr_len: bundle.pqpk_lr.len(),
        pqpk_lr_signature_len: bundle.pqpk_lr_signature.len(),
        pqpk_ot_len: bundle.pqpk_ot.as_ref().map(Vec::len),
        pqpk_ot_signature_len: bundle.pqpk_ot_signature.as_ref().map(Vec::len),
    })
}
