//! Phase 23.C — pure onboarding-shape validator lifted from
//! `commands/community/onboarding.rs`. Mirrors the sibling
//! `community_profile_validation.rs`: just MAX_* constants + a
//! single `Result<(), String>` predicate.

pub const MAX_ONBOARDING_QUESTIONS: usize = 5;
pub const MAX_ONBOARDING_OPTIONS_PER_QUESTION: usize = 10;
pub const MAX_ONBOARDING_GUIDE_STEPS: usize = 10;
pub const MAX_ONBOARDING_WELCOME_CHARS: usize = 500;
pub const MAX_ONBOARDING_QUESTION_TITLE_CHARS: usize = 100;
pub const MAX_WELCOME_SCREEN_CHANNELS: usize = 5;

pub fn validate_onboarding_shape(
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
