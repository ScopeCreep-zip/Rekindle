//! Reader-validated bounds (plan E3.0): the value ranges a governance entry
//! must respect whoever wrote it.
//!
//! These are facts about a well-formed entry, not permissions: the merge
//! drops an out-of-bounds entry before ordering (so it can neither apply
//! nor become the genesis), `validate_write` refuses it before the creator
//! bypass, `write_entry` names the refusal, and the daemon's IPC boundary
//! applies the same predicates. Each bound has one source, the constant in
//! `rekindle-types` the predicate cites; length is counted in chars by
//! [`key_format::text`], identically at every caller.

use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::governance::{
    GuideStep, OnboardingQuestion, WelcomeChannel, MAX_ONBOARDING_GUIDE_STEPS,
    MAX_ONBOARDING_ID_CHARS, MAX_ONBOARDING_OPTIONS_PER_QUESTION, MAX_ONBOARDING_QUESTIONS,
    MAX_ONBOARDING_TITLE_CHARS, MAX_ONBOARDING_WELCOME_CHARS, MAX_WELCOME_SCREEN_CHANNELS,
};
use rekindle_types::key_format::{self, KeyFormatError, MAX_DESCRIPTION_LEN};
use rekindle_types::member::MAX_TIMEOUT_SECONDS;

/// A channel's slowmode interval: `0..=MAX_SLOWMODE_SECONDS` (0 turns it off).
pub fn slowmode_ok(seconds: u32) -> bool {
    seconds <= MAX_SLOWMODE_SECONDS
}

/// A member timeout: `1..=MAX_TIMEOUT_SECONDS` (a zero timeout is no timeout).
pub fn timeout_ok(duration_seconds: u64) -> bool {
    (1..=MAX_TIMEOUT_SECONDS).contains(&duration_seconds)
}

/// A community description: [`key_format::text`] within
/// [`MAX_DESCRIPTION_LEN`] chars, no control characters but `\n` and `\t`,
/// no hidden format characters. The `Err` names why, for the IPC boundary.
pub fn description(s: &str) -> Result<&str, KeyFormatError> {
    key_format::text(s, MAX_DESCRIPTION_LEN)
}

/// [`description`] as a predicate.
pub fn description_ok(s: &str) -> bool {
    description(s).is_ok()
}

/// The field of `entry` that is out of bounds, or `None` when every bounded
/// field is within its bound. `ChannelUpdated.slowmode_seconds`,
/// `TimeoutEntry.duration_seconds`, `CommunityMeta.description`, the
/// onboarding flow and the welcome screen are bounded; every other variant
/// passes.
pub fn out_of_bounds_field(entry: &GovernanceEntry) -> Option<&'static str> {
    match entry {
        GovernanceEntry::ChannelUpdated {
            slowmode_seconds: Some(seconds),
            ..
        } if !slowmode_ok(*seconds) => Some("slowmode_seconds"),
        GovernanceEntry::TimeoutEntry {
            duration_seconds, ..
        } if !timeout_ok(*duration_seconds) => Some("duration_seconds"),
        GovernanceEntry::CommunityMeta {
            description: Some(text),
            ..
        } if !description_ok(text) => Some("description"),
        GovernanceEntry::OnboardingConfig {
            questions,
            welcome_message,
            guide_steps,
            ..
        } => onboarding_field(questions, welcome_message.as_deref(), guide_steps),
        GovernanceEntry::WelcomeScreen {
            description,
            channels,
            ..
        } => welcome_screen_field(description, channels),
        GovernanceEntry::ModReport { reason, target, .. } => mod_report_field(reason, target),
        _ => None,
    }
}

/// Whether `s` is [`key_format::text`] within `max` chars.
fn text_ok(s: &str, max: usize) -> bool {
    key_format::text(s, max).is_ok()
}

/// Whether the optional `s` is absent or [`text_ok`].
fn opt_text_ok(s: Option<&str>, max: usize) -> bool {
    s.is_none_or(|s| text_ok(s, max))
}

/// The out-of-bounds field of an onboarding flow: the counts, the ids, and
/// every text within its length with no control or hidden characters.
fn onboarding_field(
    questions: &[OnboardingQuestion],
    welcome_message: Option<&str>,
    guide_steps: &[GuideStep],
) -> Option<&'static str> {
    if questions.len() > MAX_ONBOARDING_QUESTIONS {
        return Some("questions");
    }
    if guide_steps.len() > MAX_ONBOARDING_GUIDE_STEPS {
        return Some("guide_steps");
    }
    if !opt_text_ok(welcome_message, MAX_ONBOARDING_WELCOME_CHARS) {
        return Some("welcome_message");
    }
    for q in questions {
        if !text_ok(&q.question_id, MAX_ONBOARDING_ID_CHARS) {
            return Some("question_id");
        }
        if !text_ok(&q.title, MAX_ONBOARDING_TITLE_CHARS) {
            return Some("question_title");
        }
        if !opt_text_ok(q.description.as_deref(), MAX_DESCRIPTION_LEN) {
            return Some("question_description");
        }
        if q.options.len() > MAX_ONBOARDING_OPTIONS_PER_QUESTION {
            return Some("options");
        }
        for o in &q.options {
            if !text_ok(&o.option_id, MAX_ONBOARDING_ID_CHARS) {
                return Some("option_id");
            }
            if !text_ok(&o.title, MAX_ONBOARDING_TITLE_CHARS) {
                return Some("option_title");
            }
            if !opt_text_ok(o.description.as_deref(), MAX_DESCRIPTION_LEN) {
                return Some("option_description");
            }
        }
    }
    for step in guide_steps {
        if !text_ok(&step.title, MAX_ONBOARDING_TITLE_CHARS) {
            return Some("guide_step_title");
        }
        if !description_ok(&step.description) {
            return Some("guide_step_description");
        }
    }
    None
}

/// The out-of-bounds field of a welcome screen.
fn welcome_screen_field(description: &str, channels: &[WelcomeChannel]) -> Option<&'static str> {
    if channels.len() > MAX_WELCOME_SCREEN_CHANNELS {
        return Some("channels");
    }
    if !description_ok(description) {
        return Some("description");
    }
    if channels.iter().any(|c| !description_ok(&c.description)) {
        return Some("channel_description");
    }
    None
}

/// Whether every bounded field of `entry` is within its bound.
pub fn entry_within_bounds(entry: &GovernanceEntry) -> bool {
    out_of_bounds_field(entry).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        assert!(slowmode_ok(0) && slowmode_ok(MAX_SLOWMODE_SECONDS));
        assert!(!slowmode_ok(MAX_SLOWMODE_SECONDS + 1));
        assert!(!timeout_ok(0) && timeout_ok(1) && timeout_ok(MAX_TIMEOUT_SECONDS));
        assert!(!timeout_ok(MAX_TIMEOUT_SECONDS + 1));
        assert!(description_ok(&"é".repeat(MAX_DESCRIPTION_LEN)));
        assert!(!description_ok(&"é".repeat(MAX_DESCRIPTION_LEN + 1)));
        assert!(description_ok("line\n\ttab"));
        assert!(!description_ok("bell\u{7}"));
        assert!(!description_ok("bidi\u{202e}"));
    }
}

/// A moderator report's reason and message id: [`key_format::text`]
/// within [`key_format::MAX_NOTE_LEN`].
fn mod_report_field(
    reason: &str,
    target: &rekindle_types::governance::ModReportTarget,
) -> Option<&'static str> {
    use rekindle_types::governance::ModReportTarget;
    if !text_ok(reason, key_format::MAX_NOTE_LEN) {
        return Some("reason");
    }
    match target {
        ModReportTarget::Message { message_id, .. }
            if message_id.is_empty() || !text_ok(message_id, key_format::MAX_NOTE_LEN) =>
        {
            Some("message_id")
        }
        _ => None,
    }
}
