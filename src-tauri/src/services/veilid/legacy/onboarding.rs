use std::sync::Arc;


use crate::channels::CommunityEvent;
use crate::db::DbPool;
use crate::services::cross_device_sync::{
    open_personal_sync_record, read_read_state, write_read_state,
};
use crate::state::AppState;
use rekindle_secrets::sync_key::SyncKey;

pub(crate) fn handle_peer_assisted_join(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    claimed_subkey_index: Option<u32>,
    route_blob: Option<&[u8]>,
    invite_code: Option<&str>,
) {
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.known_members.insert(pseudonym_key.to_string());
        }
    }
    tracing::info!(
        community = %community_id,
        pseudonym = %pseudonym_key,
        "peer join noted — added to known_members"
    );

    // Architecture §16 — when the join request carries a redeemable
    // invite code, hash it and bump our local uses counter for that
    // invite (the inviter's row in `community_invites`). The increment
    // is best-effort: peers without the matching row simply skip the
    // emission. The InvitesTab counter then live-updates without a
    // refetch on the inviter's window.
    if let Some(code) = invite_code {
        let code_hash = rekindle_secrets::invite::hash_invite_code(code);
        let cid = community_id.to_string();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        let app_for_emit = app_handle.clone();
        let cid_for_emit = cid.clone();
        let code_hash_for_emit = code_hash.clone();
        crate::db_helpers::db_fire(pool, "increment invite uses counter", move |conn| {
            let updated = conn.execute(
                "UPDATE community_invites SET uses = uses + 1 \
                 WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3",
                rusqlite::params![&owner_key, &cid, &code_hash],
            )?;
            if updated > 0 {
                let new_use_count: i64 = conn
                    .query_row(
                        "SELECT uses FROM community_invites \
                         WHERE owner_key = ?1 AND community_id = ?2 AND code_hash = ?3",
                        rusqlite::params![&owner_key, &cid, &code_hash],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                crate::event_dispatch::emit_live(
                    &app_for_emit,
                    "community-event",
                    &CommunityEvent::InviteUsed {
                        community_id: cid_for_emit.clone(),
                        code_hash: code_hash_for_emit.clone(),
                        new_use_count: u32::try_from(new_use_count).unwrap_or(u32::MAX),
                    },
                );
            }
            Ok(())
        });
    }

    let _ = (display_name, claimed_subkey_index, route_blob);
}

pub(crate) async fn handle_onboarding_answers(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    answers: &[rekindle_protocol::dht::community::envelope::OnboardingAnswer],
) {
    use std::collections::{HashMap, HashSet};
    use tauri::Manager;

    fn role_id_to_legacy(role_id: &rekindle_types::id::RoleId) -> u32 {
        u32::from_le_bytes([role_id.0[0], role_id.0[1], role_id.0[2], role_id.0[3]])
    }

    let Some(gov_state) = crate::state_helpers::governance_state(state, community_id) else {
        tracing::warn!(community = %community_id, "ignoring onboarding answers without governance state");
        return;
    };
    let Some(onboarding) = gov_state.onboarding.as_ref() else {
        tracing::debug!(community = %community_id, "ignoring onboarding answers without onboarding config");
        return;
    };
    if !onboarding.enabled {
        tracing::debug!(community = %community_id, "ignoring onboarding answers because onboarding is disabled");
        return;
    }

    let sender_bytes: [u8; 32] = match hex::decode(sender_pseudonym) {
        Ok(bytes) => {
            if let Ok(arr) = bytes.try_into() {
                arr
            } else {
                tracing::warn!(community = %community_id, pseudonym = %sender_pseudonym, "invalid sender pseudonym length in onboarding answers");
                return;
            }
        }
        Err(e) => {
            tracing::warn!(community = %community_id, pseudonym = %sender_pseudonym, error = %e, "invalid sender pseudonym hex in onboarding answers");
            return;
        }
    };
    let sender_key = rekindle_types::id::PseudonymKey(sender_bytes);

    let answers_by_question: HashMap<
        &str,
        &rekindle_protocol::dht::community::envelope::OnboardingAnswer,
    > = answers
        .iter()
        .map(|answer| (answer.question_id.as_str(), answer))
        .collect();
    let valid_question_ids: HashSet<&str> = onboarding
        .questions
        .iter()
        .map(|question| question.question_id.as_str())
        .collect();
    for answer in answers {
        if !valid_question_ids.contains(answer.question_id.as_str()) {
            tracing::warn!(
                community = %community_id,
                pseudonym = %sender_pseudonym,
                question_id = %answer.question_id,
                "rejecting onboarding answers because an unknown question was submitted"
            );
            return;
        }
    }

    let mut roles_to_assign = HashSet::new();
    for question in &onboarding.questions {
        let answer = answers_by_question.get(question.question_id.as_str());
        if question.required && answer.is_none() {
            tracing::warn!(
                community = %community_id,
                pseudonym = %sender_pseudonym,
                question_id = %question.question_id,
                "rejecting onboarding answers because a required question was omitted"
            );
            return;
        }
        let Some(answer) = answer else { continue };
        if question.required && answer.selected_options.is_empty() {
            tracing::warn!(
                community = %community_id,
                pseudonym = %sender_pseudonym,
                question_id = %question.question_id,
                "rejecting onboarding answers because a required question had no selection"
            );
            return;
        }
        if question.single_select && answer.selected_options.len() > 1 {
            tracing::warn!(
                community = %community_id,
                pseudonym = %sender_pseudonym,
                question_id = %question.question_id,
                "rejecting onboarding answers because a single-select question had multiple selections"
            );
            return;
        }

        let options_by_id: HashMap<&str, &_> = question
            .options
            .iter()
            .map(|option| (option.option_id.as_str(), option))
            .collect();

        for option_id in &answer.selected_options {
            let Some(option) = options_by_id.get(option_id.as_str()) else {
                tracing::warn!(
                    community = %community_id,
                    pseudonym = %sender_pseudonym,
                    question_id = %question.question_id,
                    option_id,
                    "rejecting onboarding answers because an unknown option was selected"
                );
                return;
            };
            roles_to_assign.extend(option.roles_to_assign.iter().copied());
        }
    }

    let already_assigned = gov_state
        .role_assignments
        .get(&sender_key)
        .cloned()
        .unwrap_or_default();
    let mut newly_assigned = Vec::new();
    for role_id in roles_to_assign {
        if already_assigned.contains(&role_id) {
            continue;
        }
        let entry = rekindle_types::governance::GovernanceEntry::RoleAssignment {
            target: sender_key.clone(),
            role_id,
            lamport: rekindle_utils::timestamp_ms(),
        };
        match crate::services::community::write_entry(state, community_id, entry).await {
            Ok(()) => newly_assigned.push(role_id),
            Err(e) => {
                tracing::warn!(
                    community = %community_id,
                    pseudonym = %sender_pseudonym,
                    role_id = %hex::encode(role_id.0),
                    error = %e,
                    "failed to persist onboarding role assignment"
                );
                return;
            }
        }
    }

    let final_role_ids: Vec<u32> = crate::state_helpers::governance_state(state, community_id)
        .and_then(|state| state.role_assignments.get(&sender_key).cloned())
        .map_or_else(Vec::new, |roles| {
            roles.iter().map(role_id_to_legacy).collect()
        });

    let is_self_completion = {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.member_roles
                .insert(sender_pseudonym.to_string(), final_role_ids.clone());
            if cs.my_pseudonym_key.as_deref() == Some(sender_pseudonym) {
                cs.onboarding_complete = true;
                true
            } else {
                false
            }
        } else {
            false
        }
    };

    let pool: tauri::State<'_, DbPool> = app_handle.state();
    if let Ok(owner_key) = crate::state_helpers::current_owner_key(state) {
        let cid = community_id.to_string();
        let pk = sender_pseudonym.to_string();
        let role_ids_json = serde_json::to_string(&final_role_ids).unwrap_or_default();
        crate::db_helpers::db_fire(pool.inner(), "persist onboarding completion", move |conn| {
            conn.execute(
                "UPDATE community_members SET role_ids = ?1, onboarding_complete = 1 \
                 WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                rusqlite::params![role_ids_json, owner_key, cid, pk],
            )?;
            Ok(())
        });
    }

    // Architecture §28.4 — push our own onboarding completion into the
    // personal SMPL ReadState so other paired devices stop showing the
    // wizard for this community. We only push for self-completion;
    // observing another member's completion locally is irrelevant to
    // our own ReadState. Fire-and-forget — the SMPL write may be slow
    // and the local SQLite flag already covers this device.
    if is_self_completion {
        let state_arc = Arc::clone(state);
        let pool_arc = pool.inner().clone();
        let community_id_owned = community_id.to_string();
        tokio::spawn(async move {
            if let Err(e) =
                push_onboarding_complete_to_sync(&state_arc, &pool_arc, &community_id_owned)
                    .await
            {
                tracing::warn!(community = %community_id_owned, error = %e, "failed to push onboarding completion to personal sync");
            }
        });
    }

    super::membership::handle_member_roles_changed(
        app_handle,
        state,
        community_id,
        sender_pseudonym,
        &final_role_ids,
    );

    let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
        rekindle_protocol::dht::community::envelope::ControlPayload::OnboardingComplete {
            pseudonym_key: sender_pseudonym.to_string(),
            role_ids: final_role_ids.clone(),
        },
    );
    if let Err(e) = crate::services::community::send_to_mesh(state, community_id, &envelope) {
        tracing::warn!(
            community = %community_id,
            pseudonym = %sender_pseudonym,
            error = %e,
            "failed to broadcast onboarding completion"
        );
    }

    crate::event_dispatch::emit_live(
        app_handle,
        "community-event",
        &crate::channels::CommunityEvent::OnboardingComplete {
            community_id: community_id.to_string(),
            pseudonym_key: sender_pseudonym.to_string(),
            role_ids: final_role_ids,
        },
    );

    tracing::info!(
        community = %community_id,
        pseudonym = %sender_pseudonym,
        assigned_roles = newly_assigned.len(),
        "processed onboarding answers"
    );
}

/// Architecture §28.4 — flip the `onboarding_complete[community_id]`
/// bit on the personal SMPL ReadState (subkey 1). Reads → merges →
/// writes. The merge is a logical OR so a paired device that already
/// flipped the flag won't have it cleared. No-ops gracefully when the
/// personal sync record hasn't been provisioned yet (fresh install
/// before pairing).
async fn push_onboarding_complete_to_sync(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
) -> Result<(), String> {
    let Some(handle) = open_personal_sync_record(state, pool).await else {
        return Ok(());
    };
    let master_secret = state
        .identity_secret
        .lock()
        .ok_or_else(|| "identity secret not available".to_string())?;
    let sync_key = SyncKey::from_master_secret(&master_secret);
    let mut state_doc = read_read_state(state, &handle, &sync_key)
        .await
        .unwrap_or_default();
    state_doc
        .onboarding_complete
        .insert(community_id.to_string(), true);
    write_read_state(state, &handle, &sync_key, state_doc).await?;
    Ok(())
}
