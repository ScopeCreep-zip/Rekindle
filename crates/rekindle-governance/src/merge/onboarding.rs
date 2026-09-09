//! `merge` onboarding CRDT apply rules.

use super::{GovernanceEntry, GovernanceState, OnboardingState, WelcomeScreenState};

pub(super) fn apply_onboarding(entry: &GovernanceEntry, state: &mut GovernanceState) {
    match entry {
        GovernanceEntry::OnboardingConfig {
            enabled,
            mode,
            default_channels,
            questions,
            welcome_message,
            guide_steps,
            lamport,
        } => {
            let existing_lamport = state.onboarding.as_ref().map_or(0, |o| o.lamport);
            if *lamport > existing_lamport {
                state.onboarding = Some(OnboardingState {
                    enabled: *enabled,
                    mode: mode.clone(),
                    default_channels: default_channels.clone(),
                    questions: questions.clone(),
                    welcome_message: welcome_message.clone(),
                    guide_steps: guide_steps.clone(),
                    lamport: *lamport,
                });
            }
        }

        GovernanceEntry::WelcomeScreen {
            description,
            channels,
            lamport,
        } => {
            let existing_lamport = state.welcome_screen.as_ref().map_or(0, |w| w.lamport);
            if *lamport > existing_lamport {
                state.welcome_screen = Some(WelcomeScreenState {
                    description: description.clone(),
                    channels: channels.clone(),
                    lamport: *lamport,
                });
            }
        }

        // ── Admin delete (tombstone) ──
        _ => unreachable!("apply_onboarding: unexpected variant"),
    }
}
