//! Phase 23.C — Signal session-reset orchestrators lifted from
//! `commands/friends.rs`. Implements the three P3.3 endpoints:
//! `reset_signal_session_inner` (user initiates a reset),
//! `accept_session_reset_inner` (peer accepts the reset), and
//! `decline_session_reset_inner` (peer declines).

use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

pub async fn reset_signal_session_inner(
    state: Arc<AppState>,
    pool: DbPool,
    peer_public_key: String,
) -> Result<(), String> {
    if peer_public_key.is_empty() {
        return Err("peer public key required".to_string());
    }
    if !state_helpers::is_friend(&state, &peer_public_key) {
        return Err("can only reset sessions with existing friends".to_string());
    }
    let our_prekey_bundle = {
        let signal = state.signal_manager.read();
        let handle = signal.as_ref().ok_or("Signal manager not initialized")?;
        handle
            .manager
            .delete_session(&peer_public_key)
            .map_err(|e| format!("delete_session: {e}"))?;
        let bundle = handle
            .manager
            .generate_prekey_bundle(1, Some(1), Some(1))
            .map_err(|e| format!("generate_prekey_bundle: {e}"))?;
        serde_json::to_vec(&bundle).map_err(|e| format!("serialize PreKeyBundle: {e}"))?
    };
    tracing::info!(
        peer = %peer_public_key,
        "Signal session reset by user; sending SessionResetRequest"
    );
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::SessionResetRequest {
        our_prekey_bundle,
    };
    crate::services::message_service::send_to_peer_raw(&state, &pool, &peer_public_key, &payload)
        .await
        .map_err(|e| format!("send SessionResetRequest: {e}"))?;
    Ok(())
}

pub async fn accept_session_reset_inner(
    state: Arc<AppState>,
    pool: DbPool,
    peer_public_key: String,
) -> Result<(), String> {
    if !state_helpers::is_friend(&state, &peer_public_key) {
        return Err("can only accept session reset from existing friends".to_string());
    }
    let peer_bundle_bytes = state
        .pending_session_resets
        .lock()
        .remove(&peer_public_key)
        .ok_or("no pending session reset for this peer (it may have expired or already been processed)")?;
    let peer_bundle: rekindle_crypto::signal::PreKeyBundle =
        serde_json::from_slice(&peer_bundle_bytes)
            .map_err(|e| format!("invalid PreKeyBundle in pending reset: {e}"))?;

    let session_init = {
        let signal = state.signal_manager.read();
        let handle = signal.as_ref().ok_or("Signal manager not initialized")?;
        let _ = handle.manager.delete_session(&peer_public_key);
        handle
            .manager
            .establish_session(&peer_public_key, &peer_bundle)
            .map_err(|e| format!("establish_session: {e}"))?
    };

    let our_identity_key = {
        let our_secret = (*state.identity_secret.lock()).ok_or("identity secret not loaded")?;
        let our_identity = rekindle_crypto::Identity::from_secret_bytes(&our_secret);
        our_identity.public_key_bytes().to_vec()
    };
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::SessionResetAccept {
        ephemeral_key: session_init.ephemeral_public_key,
        signed_prekey_id: session_init.signed_prekey_id,
        one_time_prekey_id: session_init.one_time_prekey_id,
        our_identity_key,
        ml_kem_ciphertext: session_init.ml_kem_ciphertext,
        used_ot_pqpk_id: session_init.used_ot_pqpk_id,
    };
    crate::services::message_service::send_to_peer_raw(&state, &pool, &peer_public_key, &payload)
        .await
        .map_err(|e| format!("send SessionResetAccept: {e}"))?;
    tracing::info!(
        peer = %peer_public_key,
        "Signal session renewal accepted; SessionResetAccept dispatched"
    );
    Ok(())
}

pub async fn decline_session_reset_inner(
    state: Arc<AppState>,
    pool: DbPool,
    peer_public_key: String,
    reason: Option<String>,
) -> Result<(), String> {
    state
        .pending_session_resets
        .lock()
        .remove(&peer_public_key);
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::SessionResetDecline {
        reason: reason.unwrap_or_default(),
    };
    let _ = crate::services::message_service::send_to_peer_raw(
        &state,
        &pool,
        &peer_public_key,
        &payload,
    )
    .await;
    Ok(())
}
