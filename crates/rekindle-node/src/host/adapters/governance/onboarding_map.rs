//! The onboarding flow and welcome screen between their IPC shape
//! (`rekindle_types::dto::onboarding`: hex channel ids, legacy `u32` role
//! ids) and their governance shape (newtype ids), on both modes (plan E3.1
//! D3). A channel id that is not 32 hex characters is refused, never
//! zero-filled.

use rekindle_governance::state::{OnboardingState, WelcomeScreenState};
use rekindle_types::dto::onboarding::{
    GuideStepDto, OnboardingConfigDto, OnboardingConfigEdit, OnboardingModeDto,
    OnboardingOptionDto, OnboardingQuestionDto, WelcomeChannelEntryDto, WelcomeScreenDto,
};
use rekindle_types::governance::{
    GovernanceEntry, GuideStep, OnboardingOption, OnboardingQuestion, WelcomeChannel,
};
use rekindle_types::id::{ChannelId, RoleId};
use rekindle_utils::random::{id_bytes_16_from, RandomSource};

/// A new onboarding question's or option's id: 16 random bytes as hex,
/// minted by the Host (plan E7.15 S3), never by the caller.
pub(crate) fn mint_id(rng: &dyn RandomSource) -> String {
    hex::encode(id_bytes_16_from(rng))
}

/// The first id `edit` names that the merged flow `current` does not hold:
/// an edit keeps an existing question's id and an existing option's id
/// within its question; a new item carries none.
pub(crate) fn unknown_id(
    edit: &OnboardingConfigEdit,
    current: Option<&OnboardingState>,
) -> Option<String> {
    let held = |id: &str| current?.questions.iter().find(|q| q.question_id == id);
    for q in &edit.questions {
        let existing = match &q.question_id {
            Some(id) => match held(id) {
                Some(existing) => Some(existing),
                None => return Some(format!("question id {id:?} is not in the flow")),
            },
            None => None,
        };
        for o in &q.options {
            let Some(id) = &o.option_id else { continue };
            if !existing.is_some_and(|e| e.options.iter().any(|eo| &eo.option_id == id)) {
                return Some(format!("option id {id:?} is not in its question"));
            }
        }
    }
    None
}

/// A channel id from its 32 hex characters.
fn channel_id(hex_str: &str) -> Result<ChannelId, String> {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| <[u8; 16]>::try_from(b).ok())
        .map(ChannelId)
        .ok_or_else(|| format!("channel id {hex_str:?} is not 32 hex characters"))
}

fn channel_ids(ids: &[String]) -> Result<Vec<ChannelId>, String> {
    ids.iter().map(|id| channel_id(id)).collect()
}

fn mode_name(mode: OnboardingModeDto) -> &'static str {
    match mode {
        OnboardingModeDto::Default => "default",
        OnboardingModeDto::Guided => "guided",
        OnboardingModeDto::Gated => "gated",
    }
}

/// The `OnboardingConfig` entry `config` writes; a new question or option
/// takes its id from `mint`.
///
/// # Errors
/// A channel id is not 32 hex characters.
pub(crate) fn config_entry(
    config: &OnboardingConfigEdit,
    mint: &mut dyn FnMut() -> String,
) -> Result<GovernanceEntry, String> {
    let questions = config
        .questions
        .iter()
        .map(|q| {
            Ok(OnboardingQuestion {
                question_id: q.question_id.clone().unwrap_or_else(&mut *mint),
                title: q.title.clone(),
                description: q.description.clone(),
                required: q.required,
                single_select: q.single_select,
                options: q
                    .options
                    .iter()
                    .map(|o| {
                        Ok(OnboardingOption {
                            option_id: o.option_id.clone().unwrap_or_else(&mut *mint),
                            title: o.title.clone(),
                            description: o.description.clone(),
                            emoji: None,
                            roles_to_assign: o
                                .roles_to_assign
                                .iter()
                                .map(|hex| {
                                    RoleId::from_hex(hex)
                                        .ok_or_else(|| format!("invalid role id: {hex}"))
                                })
                                .collect::<Result<_, String>>()?,
                            channels_to_show: channel_ids(&o.channels_to_show)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            })
        })
        .collect::<Result<_, String>>()?;
    let guide_steps = config
        .guide_steps
        .iter()
        .map(|s| {
            Ok(GuideStep {
                title: s.title.clone(),
                description: s.description.clone(),
                channel_id: s.channel_id.as_deref().map(channel_id).transpose()?,
                emoji: s.emoji.clone(),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(GovernanceEntry::OnboardingConfig {
        enabled: config.enabled,
        mode: mode_name(config.mode).to_owned(),
        default_channels: channel_ids(&config.default_channels)?,
        questions,
        welcome_message: config.welcome_message.clone(),
        guide_steps,
    })
}

/// The `WelcomeScreen` entry `screen` writes.
///
/// # Errors
/// A channel id is not 32 hex characters.
pub(crate) fn welcome_entry(screen: &WelcomeScreenDto) -> Result<GovernanceEntry, String> {
    let channels = screen
        .channels
        .iter()
        .map(|c| {
            Ok(WelcomeChannel {
                channel_id: channel_id(&c.channel_id)?,
                description: c.description.clone(),
                emoji: c.emoji.clone(),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(GovernanceEntry::WelcomeScreen {
        description: screen.description.clone(),
        channels,
    })
}

/// The merged onboarding flow as it crosses IPC.
pub(crate) fn config_view(onboarding: &OnboardingState) -> OnboardingConfigDto {
    OnboardingConfigDto {
        enabled: onboarding.enabled,
        mode: match onboarding.mode.as_str() {
            "guided" => OnboardingModeDto::Guided,
            "gated" => OnboardingModeDto::Gated,
            _ => OnboardingModeDto::Default,
        },
        default_channels: onboarding
            .default_channels
            .iter()
            .map(|id| hex::encode(id.0))
            .collect(),
        questions: onboarding
            .questions
            .iter()
            .map(|q| OnboardingQuestionDto {
                question_id: q.question_id.clone(),
                title: q.title.clone(),
                description: q.description.clone(),
                required: q.required,
                single_select: q.single_select,
                options: q
                    .options
                    .iter()
                    .map(|o| OnboardingOptionDto {
                        option_id: o.option_id.clone(),
                        title: o.title.clone(),
                        description: o.description.clone(),
                        roles_to_assign: o.roles_to_assign.iter().map(|id| id.to_hex()).collect(),
                        channels_to_show: o
                            .channels_to_show
                            .iter()
                            .map(|id| hex::encode(id.0))
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
        welcome_message: onboarding.welcome_message.clone(),
        guide_steps: onboarding
            .guide_steps
            .iter()
            .map(|s| GuideStepDto {
                title: s.title.clone(),
                description: s.description.clone(),
                channel_id: s.channel_id.as_ref().map(|id| hex::encode(id.0)),
                emoji: s.emoji.clone(),
            })
            .collect(),
    }
}

/// The merged welcome screen as it crosses IPC.
pub(crate) fn welcome_view(screen: &WelcomeScreenState) -> WelcomeScreenDto {
    WelcomeScreenDto {
        description: screen.description.clone(),
        channels: screen
            .channels
            .iter()
            .map(|c| WelcomeChannelEntryDto {
                channel_id: hex::encode(c.channel_id.0),
                description: c.description.clone(),
                emoji: c.emoji.clone(),
            })
            .collect(),
    }
}
