//! A community's onboarding and welcome-screen requests (plan E3.1 D3): the
//! community key, the reader bounds of the onboarding flow and welcome
//! screen, and the onboarding answers' counts and ids.

use rekindle_governance::validate::bounds;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::SettingsRequest;
use rekindle_types::governance::{
    GovernanceEntry, MAX_ONBOARDING_ID_CHARS, MAX_ONBOARDING_OPTIONS_PER_QUESTION,
    MAX_ONBOARDING_QUESTIONS,
};
use rekindle_types::presence::OnboardingAnswer;

use crate::host::adapters::governance::onboarding_map;

use super::{community, text, Checked};

/// An onboarding or welcome-screen request's fields.
pub(super) fn check(request: &SettingsRequest) -> Checked {
    match request {
        SettingsRequest::OnboardingConfigGet { community: key }
        | SettingsRequest::WelcomeScreenGet { community: key }
        | SettingsRequest::OnboardingComplete { community: key } => community(key),
        SettingsRequest::OnboardingConfigSet {
            community: key,
            config,
        } => community(key).and_then(|()| within_bounds(onboarding_map::config_entry(config, 0))),
        SettingsRequest::WelcomeScreenSet {
            community: key,
            screen,
        } => community(key).and_then(|()| within_bounds(onboarding_map::welcome_entry(screen, 0))),
        SettingsRequest::OnboardingSubmit {
            community: key,
            answers,
            ..
        } => community(key).and_then(|()| onboarding_answers(answers)),
        other => unreachable!("{} is not an onboarding request", other.name()),
    }
}

/// An onboarding flow or welcome screen whose channel ids parse and whose
/// every field is within the bound readers apply (`bounds`).
fn within_bounds(entry: Result<GovernanceEntry, String>) -> Checked {
    let entry = entry.map_err(|e| IpcResponse::error(400, e))?;
    match bounds::out_of_bounds_field(&entry) {
        Some(field) => Err(IpcResponse::error(400, format!("{field} is out of bounds"))),
        None => Ok(()),
    }
}

/// Onboarding answers: at most one per question the flow may ask, each
/// naming at most as many options as a question offers.
fn onboarding_answers(answers: &[OnboardingAnswer]) -> Checked {
    if answers.len() > MAX_ONBOARDING_QUESTIONS {
        return Err(IpcResponse::error(
            400,
            "onboarding answers: more answers than questions",
        ));
    }
    for answer in answers {
        text(&answer.question_id, MAX_ONBOARDING_ID_CHARS, "question id")?;
        if answer.selected_options.len() > MAX_ONBOARDING_OPTIONS_PER_QUESTION {
            return Err(IpcResponse::error(
                400,
                "onboarding answer: more options than a question offers",
            ));
        }
        for option in &answer.selected_options {
            text(option, MAX_ONBOARDING_ID_CHARS, "option id")?;
        }
    }
    Ok(())
}
