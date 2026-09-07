//! Onboarding-answers control-message handler (drained from
//! `legacy/onboarding.rs`). Validates submitted answers against the
//! governance onboarding config, assigns the configured roles via
//! `apply::write_entry`, and broadcasts completion.

use std::collections::{HashMap, HashSet};

use rekindle_protocol::dht::community::envelope::{
    CommunityEnvelope, ControlPayload, OnboardingAnswer,
};
use rekindle_types::governance::{GovernanceEntry, OnboardingQuestion};
use rekindle_types::id::{PseudonymKey, RoleId};

use crate::apply::write_entry;
use crate::event::GovernanceRuntimeEvent;
use crate::membership_events::deps::MembershipEventDeps;
use crate::membership_events::roles_changed::process_member_roles_changed;

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
            lamport: rekindle_utils::timestamp_ms(),
        };
        match write_entry(deps, community_id, entry).await {
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

    let final_role_ids: Vec<u32> = deps
        .governance_state(community_id)
        .and_then(|state| state.role_assignments.get(&sender_key).cloned())
        .map_or_else(Vec::new, |roles| {
            roles.iter().copied().map(RoleId::to_legacy_u32).collect()
        });

    let is_self = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .as_deref()
        == Some(sender_pseudonym);

    deps.persist_onboarding_completion(community_id, sender_pseudonym, &final_role_ids, is_self);
    if is_self {
        deps.push_onboarding_complete_to_sync(community_id);
    }

    process_member_roles_changed(deps, community_id, sender_pseudonym, &final_role_ids);

    let envelope = CommunityEnvelope::Control(ControlPayload::OnboardingComplete {
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
mod tests {
    use super::*;
    use rekindle_types::governance::OnboardingOption;

    fn option(option_id: &str, roles: &[u8]) -> OnboardingOption {
        OnboardingOption {
            option_id: option_id.to_string(),
            title: option_id.to_string(),
            description: None,
            emoji: None,
            roles_to_assign: roles.iter().map(|&n| RoleId([n; 16])).collect(),
            channels_to_show: Vec::new(),
        }
    }

    fn question(
        question_id: &str,
        required: bool,
        single_select: bool,
        options: Vec<OnboardingOption>,
    ) -> OnboardingQuestion {
        OnboardingQuestion {
            question_id: question_id.to_string(),
            title: question_id.to_string(),
            description: None,
            required,
            single_select,
            options,
        }
    }

    fn answer(question_id: &str, selected: &[&str]) -> OnboardingAnswer {
        OnboardingAnswer {
            question_id: question_id.to_string(),
            selected_options: selected.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn collects_roles_from_selected_options() {
        let questions = vec![question(
            "q1",
            true,
            false,
            vec![option("a", &[1]), option("b", &[2, 3])],
        )];
        let roles = validate_onboarding_answers(&questions, &[answer("q1", &["a", "b"])])
            .expect("valid answers");
        let expected: HashSet<RoleId> = [RoleId([1; 16]), RoleId([2; 16]), RoleId([3; 16])].into();
        assert_eq!(roles, expected);
    }

    #[test]
    fn rejects_unknown_question() {
        let questions = vec![question("q1", false, false, vec![option("a", &[1])])];
        assert_eq!(
            validate_onboarding_answers(&questions, &[answer("q2", &["a"])]),
            Err(OnboardingRejection::UnknownQuestion {
                question_id: "q2".to_string()
            })
        );
    }

    #[test]
    fn rejects_required_question_omitted() {
        let questions = vec![question("q1", true, false, vec![option("a", &[1])])];
        assert_eq!(
            validate_onboarding_answers(&questions, &[]),
            Err(OnboardingRejection::RequiredOmitted {
                question_id: "q1".to_string()
            })
        );
    }

    #[test]
    fn rejects_required_question_with_empty_selection() {
        let questions = vec![question("q1", true, false, vec![option("a", &[1])])];
        assert_eq!(
            validate_onboarding_answers(&questions, &[answer("q1", &[])]),
            Err(OnboardingRejection::RequiredEmpty {
                question_id: "q1".to_string()
            })
        );
    }

    #[test]
    fn rejects_single_select_with_multiple_selections() {
        let questions = vec![question(
            "q1",
            false,
            true,
            vec![option("a", &[1]), option("b", &[2])],
        )];
        assert_eq!(
            validate_onboarding_answers(&questions, &[answer("q1", &["a", "b"])]),
            Err(OnboardingRejection::SingleSelectMultiple {
                question_id: "q1".to_string()
            })
        );
    }

    #[test]
    fn rejects_unknown_option() {
        let questions = vec![question("q1", false, false, vec![option("a", &[1])])];
        assert_eq!(
            validate_onboarding_answers(&questions, &[answer("q1", &["zzz"])]),
            Err(OnboardingRejection::UnknownOption {
                question_id: "q1".to_string(),
                option_id: "zzz".to_string(),
            })
        );
    }

    #[test]
    fn optional_unanswered_question_is_skipped() {
        let questions = vec![
            question("q1", false, false, vec![option("a", &[1])]),
            question("q2", false, false, vec![option("b", &[2])]),
        ];
        let roles = validate_onboarding_answers(&questions, &[answer("q2", &["b"])])
            .expect("optional q1 may be skipped");
        assert_eq!(roles, [RoleId([2; 16])].into());
    }
}
