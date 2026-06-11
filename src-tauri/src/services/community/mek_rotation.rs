//! Phase 23.D.10 — thin facade. All MEK receive logic
//! (unwrap_received_mek + apply_received_mek + handle_incoming_mek_transfer
//! + mek_cache_has_generation) ported into `rekindle_mek_rotation::receive`
//! parameterised over `MekDistributeDeps`. Only `spawn_mek_request_with_retry`
//! stays here — it's a Tier-9 tokio::spawn wrapper that pushes
//! `RequestMEK` envelopes onto the mesh on a cascade-fall-through schedule.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::state::AppState;

pub fn spawn_mek_request_with_retry(
    state: Arc<AppState>,
    community_id: String,
    channel_id: String,
    needed_generation: u64,
    requester_pseudonym: String,
) {
    // Phase 17 — MAX_CASCADES sourced from the rekindle-mek-rotation
    // crate so the requester-side retry budget stays in lock-step with
    // the rotator-side cascade_candidates(max_cascades) ceiling. The
    // spawn task itself stays src-tauri-local (tokio::spawn against
    // AppState; not crate-side protocol logic).
    tokio::spawn(async move {
        let max_cascades = u32::try_from(rekindle_mek_rotation::MAX_CASCADES).unwrap_or(3);
        const RETRY_DEADLINE_MS: u64 = 5_000;
        // Snapshot the resolution at spawn so `0` ("send me current")
        // can detect that ANY new key landed.
        let initial_gen =
            crate::state_helpers::channel_media_mek(&state, &community_id, &channel_id)
                .map(|(_, generation)| generation);
        for cascade_index in 0..max_cascades {
            // Bail early if a satisfying MEK arrived via a concurrent
            // path (parallel rotation broadcast, an MekTransfer reply,
            // a different request that produced the same gen).
            // Satisfied means: resolution at/after the needed
            // generation — the responder may serve CURRENT when the
            // exact historical generation is gone, which still
            // converges the live stream.
            let resolved =
                crate::state_helpers::channel_media_mek(&state, &community_id, &channel_id)
                    .map(|(_, generation)| generation);
            let cache_hit = match (needed_generation, resolved) {
                (0, current) => current != initial_gen && current.is_some(),
                (needed, Some(current)) => current >= needed,
                (_, None) => false,
            };
            if cache_hit {
                return;
            }
            // Build & broadcast RequestMEK at the current cascade level.
            let request = CommunityEnvelope::Control(ControlPayload::RequestMEK {
                channel_id: channel_id.clone(),
                needed_generation,
                requester_pseudonym: requester_pseudonym.clone(),
                cascade_index,
            });
            if let Err(e) = super::send_to_mesh(&state, &community_id, &request) {
                tracing::warn!(
                    community = %community_id,
                    channel = %channel_id,
                    cascade_index,
                    error = %e,
                    "RequestMEK broadcast failed — will retry at next cascade level"
                );
            } else {
                tracing::debug!(
                    community = %community_id,
                    channel = %channel_id,
                    needed_generation,
                    cascade_index,
                    "RequestMEK sent"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_DEADLINE_MS)).await;
        }
        tracing::warn!(
            community = %community_id,
            channel = %channel_id,
            needed_generation,
            "MEK request gave up after MAX_CASCADES attempts — channel messages remain undecryptable until next rotation broadcast"
        );
    });
}

/// Phase 23.D.10 — facade around `rekindle_mek_rotation::handle_incoming_mek_transfer`.
/// Constructs a `MekAdapter` per call and delegates.
pub fn handle_incoming_mek_transfer(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
    sender_pseudonym: &str,
    wrapped_mek: &[u8],
) -> Result<u64, String> {
    let pool = tauri::Manager::try_state::<crate::db::DbPool>(app_handle)
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    let adapter =
        crate::services::mek_adapter::MekAdapter::new(Arc::clone(state), app_handle.clone(), pool);
    rekindle_mek_rotation::handle_incoming_mek_transfer(
        adapter.as_ref(),
        community_id,
        channel_id,
        sender_pseudonym,
        wrapped_mek,
    )
    .map_err(|e| e.to_string())
}
