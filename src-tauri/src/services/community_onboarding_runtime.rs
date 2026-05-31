//! Phase 23.C — onboarding-handler Tauri-runtime orchestration lifted
//! from `commands/community/onboarding.rs`. Hosts the four
//! orchestrator entry points (`set_onboarding_config_inner`,
//! `set_welcome_screen_inner`, `submit_onboarding_answers_inner`,
//! `mark_onboarding_complete_inner`) plus the three private helpers
//! (`enforce_rules_acknowledgment`, `resolve_self_assignable_roles`,
//! `my_pseudonym`).
//!
//! Per Invariant 7 these are Tauri-runtime glue around already-
//! abstracted governance-entry writes, gossip broadcasts, and
//! AppState/SQLite mutations.

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use crate::commands::community::helpers::{hex_to_id_16, require_permission};
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::services::community_onboarding_mappers::{
    governance_onboarding_to_manifest_shape, governance_welcome_to_protocol,
    onboarding_mode_to_string, protocol_guide_step_to_governance, protocol_question_to_governance,
    protocol_welcome_channel_to_governance,
};
use crate::services::community_onboarding_validation::{
    validate_onboarding_shape, MAX_WELCOME_SCREEN_CHANNELS,
};
use crate::state::SharedState;
use crate::state_helpers;

pub async fn set_onboarding_config_inner(
    state: &SharedState,
    community_id: String,
    config: serde_json::Value,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let config: rekindle_protocol::dht::community::onboarding::OnboardingConfig =
        serde_json::from_value(config).map_err(|e| format!("invalid config: {e}"))?;
    validate_onboarding_shape(&config)?;

    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::OnboardingConfig {
            enabled: config.enabled,
            mode: onboarding_mode_to_string(config.mode),
            default_channels: config
                .default_channels
                .iter()
                .map(|id| rekindle_types::id::ChannelId(hex_to_id_16(id)))
                .collect(),
            questions: config
                .questions
                .into_iter()
                .map(protocol_question_to_governance)
                .collect(),
            welcome_message: config.welcome_message,
            guide_steps: config
                .guide_steps
                .into_iter()
                .map(protocol_guide_step_to_governance)
                .collect(),
            lamport,
        },
    )
    .await
}

pub async fn set_welcome_screen_inner(
    state: &SharedState,
    community_id: String,
    screen: serde_json::Value,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let screen: rekindle_protocol::dht::community::onboarding::WelcomeScreen =
        serde_json::from_value(screen).map_err(|e| format!("invalid screen: {e}"))?;
    if screen.channels.len() > MAX_WELCOME_SCREEN_CHANNELS {
        return Err(format!(
            "welcome screen supports at most {MAX_WELCOME_SCREEN_CHANNELS} featured channels"
        ));
    }
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::WelcomeScreen {
            description: screen.description,
            channels: screen
                .channels
                .into_iter()
                .map(protocol_welcome_channel_to_governance)
                .collect(),
            lamport,
        },
    )
    .await
}

pub async fn submit_onboarding_answers_inner(
    state: &SharedState,
    community_id: String,
    answers: Vec<serde_json::Value>,
    acknowledged_rules: bool,
) -> Result<(), String> {
    enforce_rules_acknowledgment(state, &community_id, acknowledged_rules)?;
    let answers: Vec<rekindle_protocol::dht::community::envelope::OnboardingAnswer> = answers
        .into_iter()
        .map(|v| serde_json::from_value(v).map_err(|e| format!("invalid answer: {e}")))
        .collect::<Result<Vec<_>, _>>()?;

    let role_writes = resolve_self_assignable_roles(state, &community_id, &answers)?;
    let me = my_pseudonym(state, &community_id)?;
    for role_id in role_writes {
        let lamport = state_helpers::increment_lamport(state, &community_id);
        crate::services::community::write_entry(
            state,
            &community_id,
            rekindle_types::governance::GovernanceEntry::RoleAssignment {
                target: me.clone(),
                role_id,
                lamport,
            },
        )
        .await?;
    }

    let envelope = CommunityEnvelope::Control(ControlPayload::SubmitOnboardingAnswers { answers });
    crate::services::community::send_to_mesh(state, &community_id, &envelope)
}

pub async fn mark_onboarding_complete_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let pseudonym_key = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community not found")?;
        community
            .my_pseudonym_key
            .clone()
            .ok_or("no pseudonym for this community")?
    };

    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            community.onboarding_complete = true;
        }
    }

    let cid = community_id.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE community_members SET onboarding_complete = 1 \
             WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
            rusqlite::params![owner_key, cid, pseudonym_key],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

fn enforce_rules_acknowledgment(
    state: &SharedState,
    community_id: &str,
    acknowledged: bool,
) -> Result<(), String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let gov = community
        .governance_state
        .as_ref()
        .ok_or("governance state not loaded — refusing onboarding ack until merge completes")?;
    let Some(onboarding) = gov.onboarding.as_ref() else {
        return Ok(());
    };
    if onboarding.mode.eq_ignore_ascii_case("gated") && !acknowledged {
        return Err(
            "this community is gated — please acknowledge the community rules before continuing"
                .into(),
        );
    }
    Ok(())
}

fn resolve_self_assignable_roles(
    state: &SharedState,
    community_id: &str,
    answers: &[rekindle_protocol::dht::community::envelope::OnboardingAnswer],
) -> Result<Vec<rekindle_types::id::RoleId>, String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let gov = community
        .governance_state
        .as_ref()
        .ok_or("governance state not loaded")?;
    let onboarding = gov.onboarding.as_ref().ok_or("no onboarding config")?;

    let mut requested: Vec<rekindle_types::id::RoleId> = Vec::new();
    for answer in answers {
        let Some(question) = onboarding
            .questions
            .iter()
            .find(|q| q.question_id == answer.question_id)
        else {
            continue;
        };
        for option_id in &answer.selected_options {
            let Some(option) = question.options.iter().find(|o| &o.option_id == option_id) else {
                continue;
            };
            for role_id in &option.roles_to_assign {
                requested.push(*role_id);
            }
        }
    }
    requested.sort_by(|a, b| a.0.cmp(&b.0));
    requested.dedup();

    let allowed: Vec<rekindle_types::id::RoleId> = requested
        .into_iter()
        .filter(|role_id| {
            gov.roles
                .get(role_id)
                .is_some_and(|role| role.self_assignable)
        })
        .collect();
    Ok(allowed)
}

fn my_pseudonym(
    state: &SharedState,
    community_id: &str,
) -> Result<rekindle_types::id::PseudonymKey, String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let hex = community
        .my_pseudonym_key
        .as_ref()
        .ok_or("no pseudonym for this community")?;
    let bytes = hex::decode(hex).map_err(|e| format!("pseudonym hex: {e}"))?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| "pseudonym must be 32 bytes".to_string())?;
    Ok(rekindle_types::id::PseudonymKey(arr))
}

pub fn get_onboarding_config_inner(
    state: &SharedState,
    community_id: &str,
) -> Result<serde_json::Value, String> {
    let communities = state.communities.read();
    let config = communities
        .get(community_id)
        .and_then(|community| community.governance_state.as_ref())
        .and_then(|gov| gov.onboarding.as_ref())
        .map(governance_onboarding_to_manifest_shape)
        .unwrap_or_default();
    serde_json::to_value(&config).map_err(|e| format!("serialize: {e}"))
}

pub fn get_welcome_screen_inner(
    state: &SharedState,
    community_id: &str,
) -> Result<serde_json::Value, String> {
    let communities = state.communities.read();
    let screen = communities
        .get(community_id)
        .ok_or("community not found")?
        .governance_state
        .as_ref()
        .and_then(|gov| gov.welcome_screen.as_ref())
        .map(governance_welcome_to_protocol)
        .unwrap_or_default();
    serde_json::to_value(&screen).map_err(|e| format!("serialize: {e}"))
}
