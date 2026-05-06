use tauri::State;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::{hex_to_id_16, require_permission};
use super::legacy::{
    governance_onboarding_to_manifest_shape, governance_welcome_to_protocol,
    onboarding_mode_to_string, protocol_guide_step_to_governance, protocol_question_to_governance,
    protocol_welcome_channel_to_governance,
};

#[tauri::command]
pub async fn get_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<serde_json::Value, String> {
    let communities = state.communities.read();
    let config = communities
        .get(&community_id)
        .and_then(|community| community.governance_state.as_ref())
        .and_then(|gov| gov.onboarding.as_ref())
        .map(governance_onboarding_to_manifest_shape)
        .unwrap_or_default();
    serde_json::to_value(&config).map_err(|e| format!("serialize: {e}"))
}

/// Architecture §19.1 line 2520-2531 — onboarding shape caps.
const MAX_ONBOARDING_QUESTIONS: usize = 5;
const MAX_ONBOARDING_OPTIONS_PER_QUESTION: usize = 10;
const MAX_ONBOARDING_GUIDE_STEPS: usize = 10;
const MAX_ONBOARDING_WELCOME_CHARS: usize = 500;
const MAX_ONBOARDING_QUESTION_TITLE_CHARS: usize = 100;
const MAX_WELCOME_SCREEN_CHANNELS: usize = 5;

fn validate_onboarding_shape(
    config: &rekindle_protocol::dht::community::onboarding::OnboardingConfig,
) -> Result<(), String> {
    if config.questions.len() > MAX_ONBOARDING_QUESTIONS {
        return Err(format!(
            "onboarding supports at most {MAX_ONBOARDING_QUESTIONS} questions"
        ));
    }
    if config.guide_steps.len() > MAX_ONBOARDING_GUIDE_STEPS {
        return Err(format!(
            "onboarding supports at most {MAX_ONBOARDING_GUIDE_STEPS} guide steps"
        ));
    }
    if let Some(text) = config.welcome_message.as_deref() {
        if text.chars().count() > MAX_ONBOARDING_WELCOME_CHARS {
            return Err(format!(
                "welcome_message exceeds {MAX_ONBOARDING_WELCOME_CHARS} characters"
            ));
        }
    }
    for question in &config.questions {
        if question.title.chars().count() > MAX_ONBOARDING_QUESTION_TITLE_CHARS {
            return Err(format!(
                "question title exceeds {MAX_ONBOARDING_QUESTION_TITLE_CHARS} characters"
            ));
        }
        if question.options.len() > MAX_ONBOARDING_OPTIONS_PER_QUESTION {
            return Err(format!(
                "question supports at most {MAX_ONBOARDING_OPTIONS_PER_QUESTION} options"
            ));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_onboarding_config(
    state: State<'_, SharedState>,
    community_id: String,
    config: serde_json::Value,
) -> Result<(), String> {
    require_permission(&state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let config: rekindle_protocol::dht::community::onboarding::OnboardingConfig =
        serde_json::from_value(config).map_err(|e| format!("invalid config: {e}"))?;
    validate_onboarding_shape(&config)?;

    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
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

#[tauri::command]
pub async fn get_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<serde_json::Value, String> {
    let communities = state.communities.read();
    let screen = communities
        .get(&community_id)
        .ok_or("community not found")?
        .governance_state
        .as_ref()
        .and_then(|gov| gov.welcome_screen.as_ref())
        .map(governance_welcome_to_protocol)
        .unwrap_or_default();
    serde_json::to_value(&screen).map_err(|e| format!("serialize: {e}"))
}

#[tauri::command]
pub async fn set_welcome_screen(
    state: State<'_, SharedState>,
    community_id: String,
    screen: serde_json::Value,
) -> Result<(), String> {
    require_permission(&state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let screen: rekindle_protocol::dht::community::onboarding::WelcomeScreen =
        serde_json::from_value(screen).map_err(|e| format!("invalid screen: {e}"))?;
    if screen.channels.len() > MAX_WELCOME_SCREEN_CHANNELS {
        return Err(format!(
            "welcome screen supports at most {MAX_WELCOME_SCREEN_CHANNELS} featured channels"
        ));
    }
    let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
    crate::services::community::write_entry(
        state.inner(),
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

#[tauri::command]
pub async fn submit_onboarding_answers(
    state: State<'_, SharedState>,
    community_id: String,
    answers: Vec<serde_json::Value>,
    // Architecture §19.2 step 3 — when the merged OnboardingConfig is
    // in Gated mode, the joiner must locally acknowledge the community
    // rules (the welcomeMessage payload) before submitting answers.
    // None is treated as false.
    acknowledged_rules: Option<bool>,
) -> Result<(), String> {
    let acknowledged_rules = acknowledged_rules.unwrap_or(false);
    enforce_rules_acknowledgment(state.inner(), &community_id, acknowledged_rules)?;
    let answers: Vec<rekindle_protocol::dht::community::envelope::OnboardingAnswer> = answers
        .into_iter()
        .map(|v| serde_json::from_value(v).map_err(|e| format!("invalid answer: {e}")))
        .collect::<Result<Vec<_>, _>>()?;

    // Architecture §19.2 step 5: "Based on answers: client writes
    // RoleAssignment governance entries for self (self-assignable
    // roles only)." Resolve each selected option's `roles_to_assign`
    // against the merged OnboardingConfig, drop any role that isn't
    // marked `self_assignable`, then write one `RoleAssignment` per
    // surviving role. The control envelope still gets gossiped so
    // admins/moderators can see what new members chose.
    let role_writes = resolve_self_assignable_roles(state.inner(), &community_id, &answers)?;
    let me = my_pseudonym(state.inner(), &community_id)?;
    for role_id in role_writes {
        let lamport = crate::state_helpers::increment_lamport(state.inner(), &community_id);
        crate::services::community::write_entry(
            state.inner(),
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
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}

/// Plan §Failure 8 — flip the local user's `onboarding_complete` flag in
/// the `community_members` SQLite row + in-memory `CommunityState` so
/// the wizard does not re-show on the next app launch. The peer-side
/// fanout (other members marking us complete in their own SQLite rows)
/// happens via the `OnboardingComplete` control envelope already
/// dispatched from `services/veilid/legacy/onboarding.rs`. This command
/// covers the local-side persistence that mesh broadcast skips because
/// `send_to_mesh` excludes loopback.
#[tauri::command]
pub async fn mark_onboarding_complete(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
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
    db_call(pool.inner(), move |conn| {
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

/// Architecture §19.2 step 3 — when the merged `OnboardingConfig` is
/// in `Gated` mode, refuse to record onboarding answers unless the
/// joiner has acknowledged the rules. Other modes pass through
/// unchanged. We deliberately refuse if there's no governance state
/// loaded — the joiner shouldn't be able to bypass the gate by racing
/// the initial CRDT merge.
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

/// For each `OnboardingAnswer.selected_options`, look up the option in
/// the community's merged `OnboardingConfig`, collect every role the
/// option declares in `roles_to_assign`, then drop any role whose
/// `RoleDefinition` doesn't have `self_assignable: true`. The result
/// is the role set the joiner is allowed to assign themselves.
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
            let Some(option) = question.options.iter().find(|o| &o.option_id == option_id)
            else {
                continue;
            };
            for role_id in &option.roles_to_assign {
                requested.push(*role_id);
            }
        }
    }
    requested.sort_by(|a, b| a.0.cmp(&b.0));
    requested.dedup();

    // Reader-validates: drop any role that isn't self_assignable, so a
    // tampered OnboardingConfig that points to admin roles can't be
    // used to escalate (peers would also reject the entry, but
    // refusing to write it is faster + clearer).
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
