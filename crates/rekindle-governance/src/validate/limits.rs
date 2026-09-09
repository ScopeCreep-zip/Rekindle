#![allow(
    clippy::match_same_arms,
    reason = "arms are grouped by meaning, not by body: each carries the rule it encodes, and merging identical bodies would delete the comment explaining why that case exists"
)]

//! Shape/quantity limits for expressions, onboarding, and soundboard
//! metadata. Enforced at the reader-validates layer so a tampered
//! governance entry that exceeds a cap is dropped on the floor by
//! every honest peer.

use crate::state::GovernanceState;

const MAX_STATIC_EMOJIS: usize = 50;
const MAX_ANIMATED_EMOJIS: usize = 50;
const MAX_STICKERS: usize = 30;
const MAX_SOUNDBOARD_SOUNDS: usize = 48;

// Architecture §19.1 (lines 2520-2531) — onboarding shape limits.
const MAX_ONBOARDING_QUESTIONS: usize = 5;
const MAX_ONBOARDING_OPTIONS_PER_QUESTION: usize = 10;
const MAX_ONBOARDING_GUIDE_STEPS: usize = 10;
const MAX_ONBOARDING_WELCOME_CHARS: usize = 500;
const MAX_ONBOARDING_QUESTION_TITLE_CHARS: usize = 100;

pub(super) fn expression_within_limits(
    kind: &str,
    animated: bool,
    state: &GovernanceState,
) -> bool {
    let static_emoji_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "emoji" && !expr.animated)
        .count();
    let animated_emoji_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "emoji" && expr.animated)
        .count();
    let sticker_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "sticker")
        .count();
    let soundboard_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "soundboard")
        .count();

    match kind {
        "emoji" if animated => animated_emoji_count < MAX_ANIMATED_EMOJIS,
        "emoji" => static_emoji_count < MAX_STATIC_EMOJIS,
        "sticker" => sticker_count < MAX_STICKERS,
        "soundboard" => soundboard_count < MAX_SOUNDBOARD_SOUNDS,
        _ => false,
    }
}

/// Architecture §19.1 line 2520-2531 — onboarding shape caps. We
/// enforce these at the validate layer (in addition to the Tauri
/// command pre-flight) so a tampered governance entry that exceeds
/// limits is dropped on the floor by every honest peer.
pub(super) fn onboarding_within_limits(
    mode: &str,
    questions: &[rekindle_types::governance::OnboardingQuestion],
    welcome_message: Option<&str>,
    guide_steps: &[rekindle_types::governance::GuideStep],
) -> bool {
    if !matches!(mode, "default" | "guided" | "gated") {
        return false;
    }
    if questions.len() > MAX_ONBOARDING_QUESTIONS {
        return false;
    }
    if guide_steps.len() > MAX_ONBOARDING_GUIDE_STEPS {
        return false;
    }
    if let Some(text) = welcome_message {
        if text.chars().count() > MAX_ONBOARDING_WELCOME_CHARS {
            return false;
        }
    }
    for question in questions {
        if question.title.chars().count() > MAX_ONBOARDING_QUESTION_TITLE_CHARS {
            return false;
        }
        if question.options.len() > MAX_ONBOARDING_OPTIONS_PER_QUESTION {
            return false;
        }
    }
    true
}

/// Architecture §18.3 — soundboard entries must carry valid duration /
/// volume / emoji metadata. Emoji and sticker entries must NOT carry it
/// (any peer that smuggles `sound_meta` onto a non-soundboard entry has
/// produced an invalid wire object — drop it on the floor).
pub(super) fn soundboard_meta_valid(
    kind: &str,
    sound_meta: Option<&rekindle_types::expression::SoundboardMeta>,
) -> bool {
    use rekindle_types::expression::SoundboardMeta;
    match (kind, sound_meta) {
        ("soundboard", Some(meta)) => {
            SoundboardMeta::validate_duration(meta.duration_seconds).is_ok()
                && SoundboardMeta::validate_volume(meta.volume).is_ok()
                && SoundboardMeta::validate_emoji(meta.emoji.as_deref()).is_ok()
        }
        // Legacy soundboard entries (pre-meta) are still acceptable —
        // peers display them with default volume 1.0 and skip duration
        // enforcement until the entry is rewritten by the uploader.
        ("soundboard", None) => true,
        // Emoji / sticker entries must not carry sound_meta.
        (_, None) => true,
        (_, Some(_)) => false,
    }
}
