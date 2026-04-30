use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;

pub(crate) fn handle_peer_assisted_join(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    claimed_subkey_index: Option<u32>,
    route_blob: Option<&[u8]>,
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
    use tauri::{Emitter, Manager};

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

    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.member_roles
                .insert(sender_pseudonym.to_string(), final_role_ids.clone());
            if cs.my_pseudonym_key.as_deref() == Some(sender_pseudonym) {
                cs.onboarding_complete = true;
            }
        }
    }

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

    let _ = app_handle.emit(
        "community-event",
        crate::channels::CommunityEvent::OnboardingComplete {
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
