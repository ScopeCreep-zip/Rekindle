//! `merge` onboarding CRDT apply rules.

use crate::op::OpStamp;

use super::{GovernanceEntry, GovernanceState, OnboardingState, WelcomeScreenState};

pub(super) fn apply_onboarding(entry: &GovernanceEntry, stamp: OpStamp, state: &mut GovernanceState) {
    match entry {
        GovernanceEntry::OnboardingConfig {
            enabled,
            mode,
            default_channels,
            questions,
            welcome_message,
            guide_steps,
        } => {
            let existing_stamp = state.onboarding.as_ref().map_or(OpStamp::MIN, |o| o.stamp);
            if stamp > existing_stamp {
                state.onboarding = Some(OnboardingState {
                    enabled: *enabled,
                    mode: mode.clone(),
                    default_channels: default_channels.clone(),
                    questions: questions.clone(),
                    welcome_message: welcome_message.clone(),
                    guide_steps: guide_steps.clone(),
                    stamp,
                });
            }
        }

        GovernanceEntry::WelcomeScreen {
            description,
            channels,
        } => {
            let existing_stamp = state.welcome_screen.as_ref().map_or(OpStamp::MIN, |w| w.stamp);
            if stamp > existing_stamp {
                state.welcome_screen = Some(WelcomeScreenState {
                    description: description.clone(),
                    channels: channels.clone(),
                    stamp,
                });
            }
        }

        // ── Admin delete (tombstone) ──
        _ => unreachable!("apply_onboarding: unexpected variant"),
    }
}
