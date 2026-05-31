//! Phase 23.C — split from friend_runtime.rs. setup_invite_contact body.

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

/// Cache the route blob from an invite for immediate contact +
/// establish a Signal session from the invite's prekey bundle +
/// refresh the route blob from the peer's mailbox if available.
///
/// Called from `add_friend_from_invite`. All operations are
/// best-effort: failures log but don't block invite acceptance.
pub async fn setup_invite_contact(
    state: &Arc<AppState>,
    blob: &rekindle_protocol::messaging::envelope::InviteBlob,
) {
    // Cache the route blob from the invite for immediate contact
    tracing::info!(
        peer = %blob.public_key,
        route_blob_len = blob.route_blob.len(),
        route_count = blob.route_blob.first().copied().unwrap_or(0),
        route_blob_hex_preview = %hex::encode(&blob.route_blob[..blob.route_blob.len().min(32)]),
        "setup_invite_contact: received route blob from invite"
    );
    let api = state_helpers::veilid_api(state);
    if let Some(ref api) = api {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.manager
                .cache_route(api, &blob.public_key, blob.route_blob.clone());
        }
    } else {
        tracing::warn!("setup_invite_contact: no veilid API available — cannot cache route");
    }

    // Establish Signal session from invite's PreKeyBundle
    // Clear any stale session first (e.g., from a previous friendship that was removed)
    if let Ok(bundle) =
        serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(&blob.prekey_bundle)
    {
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            let _ = handle.manager.delete_session(&blob.public_key);
            match handle.manager.establish_session(&blob.public_key, &bundle) {
                Ok(_init_info) => {
                    tracing::info!(peer = %blob.public_key, "established Signal session from invite");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to establish Signal session from invite");
                }
            }
        }
    }

    // Try reading the peer's mailbox for a fresh route blob (invite may be stale)
    let rc = state
        .node
        .read()
        .as_ref()
        .map(|nh| nh.routing_context.clone());
    if let Some(rc) = rc {
        match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(&rc, &blob.mailbox_dht_key)
            .await
        {
            Ok(Some(fresh_blob)) if !fresh_blob.is_empty() => {
                if let Some(ref api) = api {
                    let mut dht_mgr = state.dht_manager.write();
                    if let Some(mgr) = dht_mgr.as_mut() {
                        mgr.manager.cache_route(api, &blob.public_key, fresh_blob);
                    }
                }
                tracing::debug!("refreshed route blob from peer's mailbox");
            }
            _ => tracing::trace!("no fresh route blob in peer mailbox — using invite blob"),
        }
    }
}

