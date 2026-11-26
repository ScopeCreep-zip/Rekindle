//! Onboarding-answers control-message handler (drained from
//! `legacy/onboarding.rs`). Validates submitted answers against the
//! governance onboarding config, assigns the configured roles via
//! `apply::write_entry_queued` (it runs on the ingest loop, which must not
//! wait on the network: the outbox lands the ops), and broadcasts completion.

use std::collections::{HashMap, HashSet};

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, OnboardingAnswer};
use rekindle_types::governance::{GovernanceEntry, OnboardingQuestion};
use rekindle_types::id::{PseudonymKey, RoleId};

use crate::apply::write_entry_queued;
use crate::event::GovernanceRuntimeEvent;
use crate::membership_events::deps::MembershipEventDeps;

/// Why an onboarding submission failed validation. Carries enough detail
/// for the handler to log the same context the legacy per-check warnings
/// did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingRejection {
    /// An answer referenced a question that isn't in the config.
    UnknownQuestion { question_id: String },
    /// A required question had no answer submitted.
    RequiredOmitted { question_id: String },
    /// A required question was answered with an empty selection.
    RequiredEmpty { question_id: String },
    /// A single-select question carried more than one selection.
    SingleSelectMultiple { question_id: String },
    /// An answer selected an option that isn't on the question.
    UnknownOption {
        question_id: String,
        option_id: String,
    },
}

/// Validate `answers` against the onboarding `questions` and collect the
/// roles those answers grant. Pure — no I/O, no state — so it is unit
/// tested directly. Returns the deduped role set, or the first rejection
/// encountered (matching the legacy short-circuit order).
pub fn validate_onboarding_answers(
    questions: &[OnboardingQuestion],
    answers: &[OnboardingAnswer],
) -> Result<HashSet<RoleId>, OnboardingRejection> {
    let answers_by_question: HashMap<&str, &OnboardingAnswer> = answers
        .iter()
        .map(|answer| (answer.question_id.as_str(), answer))
        .collect();
    let valid_question_ids: HashSet<&str> = questions
        .iter()
        .map(|question| question.question_id.as_str())
        .collect();
    for answer in answers {
        if !valid_question_ids.contains(answer.question_id.as_str()) {
            return Err(OnboardingRejection::UnknownQuestion {
                question_id: answer.question_id.clone(),
            });
        }
    }

    let mut roles_to_assign = HashSet::new();
    for question in questions {
        let answer = answers_by_question.get(question.question_id.as_str());
        if question.required && answer.is_none() {
            return Err(OnboardingRejection::RequiredOmitted {
                question_id: question.question_id.clone(),
            });
        }
        let Some(answer) = answer else { continue };
        if question.required && answer.selected_options.is_empty() {
            return Err(OnboardingRejection::RequiredEmpty {
                question_id: question.question_id.clone(),
            });
        }
        if question.single_select && answer.selected_options.len() > 1 {
            return Err(OnboardingRejection::SingleSelectMultiple {
                question_id: question.question_id.clone(),
            });
        }

        let options_by_id: HashMap<&str, &_> = question
            .options
            .iter()
            .map(|option| (option.option_id.as_str(), option))
            .collect();
        for option_id in &answer.selected_options {
            let Some(option) = options_by_id.get(option_id.as_str()) else {
                return Err(OnboardingRejection::UnknownOption {
                    question_id: question.question_id.clone(),
                    option_id: option_id.clone(),
                });
            };
            roles_to_assign.extend(option.roles_to_assign.iter().copied());
        }
    }
    Ok(roles_to_assign)
}

pub async fn process_onboarding_answers<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    sender_pseudonym: &str,
    answers: &[OnboardingAnswer],
) {
    let Some(gov_state) = deps.governance_state(community_id) else {
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

    let Ok(sender_bytes) = hex::decode(sender_pseudonym) else {
        tracing::warn!(community = %community_id, pseudonym = %sender_pseudonym, "invalid sender pseudonym hex in onboarding answers");
        return;
    };
    let Ok(sender_arr): Result<[u8; 32], _> = sender_bytes.try_into() else {
        tracing::warn!(community = %community_id, pseudonym = %sender_pseudonym, "invalid sender pseudonym length in onboarding answers");
        return;
    };
    let sender_key = PseudonymKey(sender_arr);

    let roles_to_assign = match validate_onboarding_answers(&onboarding.questions, answers) {
        Ok(roles) => roles,
        Err(rejection) => {
            tracing::warn!(
                community = %community_id,
                pseudonym = %sender_pseudonym,
                reason = ?rejection,
                "rejecting onboarding answers because validation failed"
            );
            return;
        }
    };

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
        let entry = GovernanceEntry::RoleAssignment {
            target: sender_key.clone(),
            role_id,
        };
        match write_entry_queued(deps, community_id, entry).await {
            Ok(_) => newly_assigned.push(role_id),
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

    let mut final_role_ids: Vec<RoleId> = deps
        .governance_state(community_id)
        .and_then(|state| state.role_assignments.get(&sender_key).cloned())
        .map_or_else(Vec::new, |roles| roles.into_iter().collect());
    final_role_ids.sort_unstable_by_key(|id| id.0);

    let is_self = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .as_deref()
        == Some(sender_pseudonym);

    deps.persist_onboarding_completion(community_id, sender_pseudonym, is_self);
    if is_self {
        deps.push_onboarding_complete_to_sync(community_id);
    }

    let envelope = CommunityEnvelope::control(ControlPayload::OnboardingComplete {
        pseudonym_key: sender_pseudonym.to_string(),
        role_ids: final_role_ids.clone(),
    });
    if let Err(e) = deps.send_to_mesh(community_id, &envelope) {
        tracing::warn!(
            community = %community_id,
            pseudonym = %sender_pseudonym,
            error = %e,
            "failed to broadcast onboarding completion"
        );
    }

    deps.emit_event(GovernanceRuntimeEvent::OnboardingComplete {
        community_id: community_id.to_string(),
        pseudonym_hex: sender_pseudonym.to_string(),
        role_ids: final_role_ids,
    });

    tracing::info!(
        community = %community_id,
        pseudonym = %sender_pseudonym,
        assigned_roles = newly_assigned.len(),
        "processed onboarding answers"
    );
}

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod tests;
