//! The onboarding flow and welcome screen between their IPC shape
//! (`rekindle_types::dto::onboarding`: hex channel ids, legacy `u32` role
//! ids) and their governance shape (newtype ids), on both modes (plan E3.1
//! D3). A channel id that is not 32 hex characters is refused, never
//! zero-filled.

use rekindle_governance::state::{OnboardingState, WelcomeScreenState};
use rekindle_types::dto::onboarding::{
    GuideStepDto, OnboardingConfigDto, OnboardingModeDto, OnboardingOptionDto,
    OnboardingQuestionDto, WelcomeChannelEntryDto, WelcomeScreenDto,
};
use rekindle_types::governance::{
    GovernanceEntry, GuideStep, OnboardingOption, OnboardingQuestion, WelcomeChannel,
};
use rekindle_types::id::{ChannelId, RoleId};

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

/// The `OnboardingConfig` entry `config` writes.
///
/// # Errors
/// A channel id is not 32 hex characters.
pub(crate) fn config_entry(config: &OnboardingConfigDto) -> Result<GovernanceEntry, String> {
    let questions = config
        .questions
        .iter()
        .map(|q| {
            Ok(OnboardingQuestion {
                question_id: q.question_id.clone(),
                title: q.title.clone(),
                description: q.description.clone(),
                required: q.required,
                single_select: q.single_select,
                options: q
                    .options
                    .iter()
                    .map(|o| {
                        Ok(OnboardingOption {
                            option_id: o.option_id.clone(),
                            title: o.title.clone(),
                            description: o.description.clone(),
                            emoji: None,
                            roles_to_assign: o
                                .roles_to_assign
                                .iter()
                                .copied()
                                .map(RoleId::from_legacy_u32)
                                .collect(),
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
                        roles_to_assign: o
                            .roles_to_assign
                            .iter()
                            .copied()
                            .map(RoleId::to_legacy_u32)
                            .collect(),
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
