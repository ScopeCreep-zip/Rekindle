//! Coordinator-side onboarding logic.
//!
//! When a new member joins, the coordinator checks the community's onboarding
//! configuration. In `Default` mode, the member is admitted immediately. In
//! `Guided` or `Gated` mode, the coordinator sends onboarding questions to
//! the new member and waits for answers before completing the join.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{ControlPayload, OnboardingAnswer};
use rekindle_protocol::dht::community::manifest;
use rekindle_protocol::dht::community::onboarding::{OnboardingConfig, OnboardingMode};
use rekindle_protocol::dht::DHTManager;

use crate::state::AppState;
use crate::state_helpers;

/// Process a join request through the onboarding flow.
///
/// Returns `Ok(Some(payload))` with `OnboardingQuestions` if the member needs
/// to complete onboarding, or `Ok(None)` if they can be admitted immediately.
pub async fn check_onboarding(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<Option<ControlPayload>, String> {
    let config = read_onboarding_config(state, community_id).await?;

    if !config.enabled || config.mode == OnboardingMode::Default {
        return Ok(None);
    }

    // Guided or Gated mode: send questions to the new member
    let questions: Vec<serde_json::Value> = config
        .questions
        .iter()
        .map(|q| serde_json::to_value(q).unwrap_or_default())
        .collect();

    Ok(Some(ControlPayload::OnboardingQuestions { questions }))
}

/// Process onboarding answers from a member.
///
/// Resolves selected options to role assignments and channel visibility.
/// Returns the list of role IDs to assign based on selected options.
pub async fn process_answers(
    state: &Arc<AppState>,
    community_id: &str,
    answers: &[OnboardingAnswer],
) -> Result<Vec<u32>, String> {
    let config = read_onboarding_config(state, community_id).await?;

    let mut roles_to_assign = Vec::new();

    for answer in answers {
        // Find the matching question
        let question = config
            .questions
            .iter()
            .find(|q| q.question_id == answer.question_id);

        let Some(question) = question else {
            continue;
        };

        // Validate required questions have answers
        if question.required && answer.selected_options.is_empty() {
            return Err(format!(
                "question '{}' is required but has no answers",
                question.question_id
            ));
        }

        // Collect roles from selected options
        for opt_id in &answer.selected_options {
            if let Some(opt) = question.options.iter().find(|o| o.option_id == *opt_id) {
                roles_to_assign.extend_from_slice(&opt.roles_to_assign);
            }
        }
    }

    // Deduplicate role IDs
    roles_to_assign.sort_unstable();
    roles_to_assign.dedup();

    Ok(roles_to_assign)
}

/// Read the onboarding config from the manifest.
async fn read_onboarding_config(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<OnboardingConfig, String> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc);

    let manifest_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.manifest_key.clone().or_else(|| Some(c.id.clone())))
            .ok_or("no manifest key")?
    };

    Ok(manifest::read_onboarding(&mgr, &manifest_key)
        .await
        .map_err(|e| format!("read onboarding: {e}"))?
        .unwrap_or_default())
}
