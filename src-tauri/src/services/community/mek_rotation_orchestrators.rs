//! Phase 23.D.11 — thin facade. The rotator-initiated rotation
//! (rotate_text_mek_for_departure) is ported into `rekindle_mek_rotation::rotate` parameterised over
//! `MekDistributeDeps`. `handle_request_mek` stays here as a Tier-9
//! facade because it dispatches local `selected_request_responder`
//! gating before delegating distribute.

use std::sync::Arc;

use rekindle_mek_rotation::MekDistributeDeps;
use rekindle_secrets::rotator::{cascade_candidates, select_mek_responder};
use rekindle_types::channel_keys::KeyScope;

use crate::services::mek_adapter::MekAdapter;
use crate::state::AppState;

use super::mek_rotation_support::{my_pseudonym, pseudonym_from_hex};

fn build_adapter(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
) -> Result<Arc<MekAdapter>, String> {
    let pool = state.db.current()?;
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
    // `channel_id` is the RequestMEK wire field: a channel's id, or empty
    // for the community key.
    let scope = KeyScope::from_wire(Some(channel_id))
        .ok_or_else(|| format!("RequestMEK names no key scope: {channel_id:?}"))?;
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

    // `needed_generation == 0` is the wire sentinel for "send me your
    // current generation" (session-join acquisition). Otherwise serve
    // the EXACT generation (cache or per-generation keystore history);
    // when the exact one is gone but our current is newer, serve
    // current — the requester treats >= needed as satisfied, so the
    // live stream converges instead of the request cascading to
    // nothing forever.
    // Full key (with provenance) — serving a `from_bytes` reconstruction would
    // strip the minter's election rank, letting the requester later flip to a
    // non-canonical same-generation key.
    let current = adapter.cache().current(community_id, scope);
    let mek = if needed_generation == 0 {
        current.ok_or_else(|| {
            format!("no current MEK for community {community_id} channel {channel_id}")
        })?
    } else {
        super::mek_rotation_support::lookup_mek(
            state,
            community_id,
            scope,
            needed_generation,
        )
        .or_else(|| current.filter(|mek| mek.generation() > needed_generation))
        .ok_or_else(|| {
            format!(
                "no MEK at generation {needed_generation} (nor newer) for community {community_id} channel {channel_id}"
            )
        })?
    };

    let requester_route = adapter
        .as_ref()
        .online_recipients(community_id, None)
        .into_iter()
        .find(|r| r.pseudonym_hex == requester_pseudonym)
        .ok_or("requester not in online peer set")?;

    rekindle_mek_rotation::distribute_mek(
        adapter.as_ref(),
        community_id,
        scope,
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

/// A member left the community — kicked, banned or departed: replace the
/// keys they hold. The community key (plan D20; architecture §10.5), and
/// our call-media key when they share our voice channel (plan C7.20).
/// Fire-and-forget: the removal itself has already happened and must not
/// wait on distribution.
pub fn spawn_departure_rotations(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    departed_pseudonym: &str,
) {
    crate::services::voice_signaling_adapter::member_departed(
        app_handle,
        state,
        community_id,
        departed_pseudonym,
    );
    let app_handle = app_handle.clone();
    let state = Arc::clone(state);
    let community_id = community_id.to_string();
    let departed = departed_pseudonym.to_string();
    crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop("departure MEK rotation", async move {
        if let Err(error) =
            rotate_text_mek_for_departure(&app_handle, &state, &community_id, &departed).await
        {
            tracing::debug!(community = %community_id, member = %departed, %error, "community MEK rotation skipped after departure");
        }
    });
}
