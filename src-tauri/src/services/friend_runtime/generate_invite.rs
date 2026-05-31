//! Phase 23.C — generate_invite orchestrator lifted from
//! `commands/friends.rs`. Gather identity + profile DHT keys
//! (with 30s wait for DHT publish), validate route blob is
//! self-importable, generate PreKeyBundle, create signed invite
//! blob with B11 signature-covered issued_at, encode URL, persist
//! to outgoing_invites for tracking.

use std::sync::Arc;

use serde::Serialize;

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateInviteResult {
    pub url: String,
    pub invite_id: String,
}

pub async fn generate_invite_inner(
    state: Arc<AppState>,
    pool: DbPool,
) -> Result<GenerateInviteResult, String> {
    let (public_key, display_name, secret_key) = {
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set")?;
        let pk = id.public_key.clone();
        let dn = id.display_name.clone();
        let sk = *state.identity_secret.lock();
        let sk = sk.ok_or("signing key not initialized")?;
        (pk, dn, sk)
    };

    let (profile_dht_key, route_blob, mailbox_dht_key) =
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                match state_helpers::profile_dht_info(&state) {
                    Ok(info) => return info,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
                }
            }
        })
        .await
        .map_err(|_| "Network not ready — please wait a moment and try again".to_string())?;

    tracing::info!(
        route_blob_len = route_blob.len(),
        route_count = route_blob.first().copied().unwrap_or(0),
        route_blob_hex_preview = %hex::encode(&route_blob[..route_blob.len().min(32)]),
        profile_dht_key = %profile_dht_key,
        mailbox_dht_key = %mailbox_dht_key,
        "generate_invite: route blob from state"
    );

    if let Some(api) = state_helpers::veilid_api(&state) {
        match api.import_remote_private_route(route_blob.clone()) {
            Ok(_) => tracing::info!("generate_invite: route blob self-import OK"),
            Err(e) => {
                tracing::error!(error = %e, "generate_invite: OUR OWN route blob fails to import!");
                return Err(format!("route blob is invalid: {e}"));
            }
        }
    }

    let prekey_bundle = {
        let signal = state.signal_manager.read();
        let handle = signal.as_ref().ok_or("signal manager not initialized")?;
        let bundle = handle
            .manager
            .generate_prekey_bundle(1, Some(1), Some(1))
            .map_err(|e| format!("generate prekey bundle: {e}"))?;
        serde_json::to_vec(&bundle).map_err(|e| format!("serialize prekey bundle: {e}"))?
    };

    let invite_id = uuid::Uuid::new_v4().to_string();

    let issued_at_ms = rekindle_utils::timestamp_ms();
    let blob = rekindle_protocol::messaging::create_invite_blob(
        &secret_key,
        &public_key,
        &display_name,
        &mailbox_dht_key,
        &profile_dht_key,
        &route_blob,
        &prekey_bundle,
        Some(&invite_id),
        issued_at_ms,
    );
    let url = rekindle_protocol::messaging::encode_invite_url(&blob);

    let owner_key = state_helpers::current_owner_key(&state)?;
    crate::invite_helpers::create_outgoing_invite(&pool, &owner_key, &invite_id, &url).await?;

    tracing::info!(%invite_id, "generated tracked invite");
    Ok(GenerateInviteResult { url, invite_id })
}
