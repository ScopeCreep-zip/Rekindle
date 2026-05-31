//! Phase 23.D.11 — thin facade. Both rotator-initiated rotations
//! (rotate_text_mek_for_departure + rotate_voice_mek_for_membership)
//! ported into `rekindle_mek_rotation::rotate` parameterised over
//! `MekDistributeDeps`. `handle_request_mek` stays here as a Tier-9
//! facade because it dispatches local `selected_request_responder`
//! gating before delegating distribute.

use std::sync::Arc;

use rekindle_mek_rotation::MekDistributeDeps;
use rekindle_secrets::rotator::{cascade_candidates, select_mek_responder};

use crate::services::mek_adapter::MekAdapter;
use crate::state::AppState;

use super::mek_rotation_support::{my_pseudonym, pseudonym_from_hex};

fn build_adapter(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
) -> Result<Arc<MekAdapter>, String> {
    let pool = tauri::Manager::try_state::<crate::db::DbPool>(app_handle)
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    Ok(MekAdapter::new(Arc::clone(state), app_handle.clone(), pool))
}

pub(super) fn selected_request_responder(
    state: &Arc<AppState>,
    community_id: &str,
    requester_pseudonym: &str,
    candidates: &[String],
    cascade_index: u32,
) -> Result<bool, String> {
    let mut candidate_keys = candidates
        .iter()
        .filter_map(|pseudonym| pseudonym_from_hex(pseudonym))
        .collect::<Vec<_>>();
    if let Some(me) = my_pseudonym(state, community_id) {
        candidate_keys.push(me);
    }
    let requester = pseudonym_from_hex(requester_pseudonym).ok_or("invalid requester pseudonym")?;
    if cascade_index == 0 {
        let Some(selected) = select_mek_responder(&requester, &candidate_keys) else {
            return Ok(false);
        };
        return Ok(my_pseudonym(state, community_id).as_ref() == Some(&selected));
    }
    let cascade = cascade_candidates(
        &requester,
        &candidate_keys
            .iter()
            .filter(|m| *m != &requester)
            .cloned()
            .collect::<Vec<_>>(),
        cascade_index as usize,
    );
    let Some(selected) = cascade.get(cascade_index as usize) else {
        return Ok(false);
    };
    Ok(my_pseudonym(state, community_id).as_ref() == Some(selected))
}

pub async fn rotate_text_mek_for_departure(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    departed_pseudonym: &str,
) -> Result<(), String> {
    let adapter = build_adapter(app_handle, state)?;
    rekindle_mek_rotation::rotate_text_mek_for_departure(
        adapter.as_ref(),
        community_id,
        departed_pseudonym,
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn rotate_voice_mek_for_membership(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    trigger_pseudonym: &str,
    include_trigger_in_recipients: bool,
) -> Result<(), String> {
    let adapter = build_adapter(app_handle, state)?;
    rekindle_mek_rotation::rotate_voice_mek_for_membership(
        adapter.as_ref(),
        community_id,
        channel_id,
        trigger_pseudonym,
        include_trigger_in_recipients,
    )
    .await
    .map_err(|e| e.to_string())
}

/// Responder-side: when our peer asked the mesh for a MEK at a given
/// generation, gate on `selected_request_responder` to decide whether
/// WE are the elected respondent for this cascade index. If so, look
/// up the MEK and call `distribute_mek` directly at the requester.
pub async fn handle_request_mek(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    needed_generation: u64,
    requester_pseudonym: &str,
    cascade_index: u32,
) -> Result<(), String> {
    let adapter = build_adapter(app_handle, state)?;

    let candidates: Vec<String> = adapter
        .as_ref()
        .online_recipients(community_id, None)
        .into_iter()
        .map(|r| r.pseudonym_hex)
        .collect();
    if !selected_request_responder(
        state,
        community_id,
        requester_pseudonym,
        &candidates,
        cascade_index,
    )? {
        return Ok(());
    }

    let mek = super::mek_rotation_support::lookup_mek(
        app_handle,
        state,
        community_id,
        channel_id,
        needed_generation,
    )
    .ok_or_else(|| {
        format!(
            "no MEK at generation {needed_generation} for community {community_id} channel {channel_id}"
        )
    })?;

    let requester_route = adapter
        .as_ref()
        .online_recipients(community_id, None)
        .into_iter()
        .find(|r| r.pseudonym_hex == requester_pseudonym)
        .ok_or("requester not in online peer set")?;

    rekindle_mek_rotation::distribute_mek(
        adapter.as_ref(),
        community_id,
        if channel_id.is_empty() {
            None
        } else {
            Some(channel_id)
        },
        &mek,
        &[rekindle_mek_rotation::RotationRecipient {
            pseudonym_hex: requester_pseudonym.to_string(),
            route_blob: requester_route.route_blob,
        }],
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
