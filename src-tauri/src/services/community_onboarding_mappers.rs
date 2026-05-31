//! Phase 23.C — onboarding DTO mappers lifted from
//! `commands/community/legacy/onboarding.rs`. Pure conversions
//! between protocol-shaped types (DHT/wire) and governance-shaped
//! types (in-memory state + governance log entries). No I/O, no
//! AppState. Used by `community_onboarding_runtime` (write path) and
//! `commands::community::onboarding` (read path).

use crate::commands::community::helpers::{hex_to_id_16, u32_to_role_id};

pub fn governance_onboarding_to_manifest_shape(
    onboarding: &rekindle_governance::state::OnboardingState,
) -> rekindle_protocol::dht::community::onboarding::OnboardingConfig {
    use rekindle_protocol::dht::community::onboarding::{OnboardingConfig, OnboardingMode};

    OnboardingConfig {
        enabled: onboarding.enabled,
        mode: match onboarding.mode.as_str() {
            "guided" => OnboardingMode::Guided,
            "gated" => OnboardingMode::Gated,
            _ => OnboardingMode::Default,
        },
        default_channels: onboarding
            .default_channels
            .iter()
            .map(|id| hex::encode(id.0))
            .collect(),
        questions: onboarding
            .questions
            .iter()
            .map(governance_question_to_protocol)
            .collect(),
        welcome_message: onboarding.welcome_message.clone(),
        guide_steps: onboarding
            .guide_steps
            .iter()
            .map(governance_guide_step_to_protocol)
            .collect(),
    }
}

pub fn onboarding_mode_to_string(
    mode: rekindle_protocol::dht::community::onboarding::OnboardingMode,
) -> String {
    match mode {
        rekindle_protocol::dht::community::onboarding::OnboardingMode::Default => "default",
        rekindle_protocol::dht::community::onboarding::OnboardingMode::Guided => "guided",
        rekindle_protocol::dht::community::onboarding::OnboardingMode::Gated => "gated",
    }
    .to_string()
}

pub fn governance_welcome_to_protocol(
    screen: &rekindle_governance::state::WelcomeScreenState,
) -> rekindle_protocol::dht::community::onboarding::WelcomeScreen {
    use rekindle_protocol::dht::community::onboarding::{WelcomeChannelEntry, WelcomeScreen};

    let channels = screen
        .channels
        .iter()
        .map(|channel| WelcomeChannelEntry {
            channel_id: hex::encode(channel.channel_id.0),
            description: channel.description.clone(),
            emoji: channel.emoji.clone(),
        })
        .collect();

    WelcomeScreen {
        description: screen.description.clone(),
        channels,
    }
}

pub fn protocol_question_to_governance(
    question: rekindle_protocol::dht::community::onboarding::OnboardingQuestion,
) -> rekindle_types::governance::OnboardingQuestion {
    rekindle_types::governance::OnboardingQuestion {
        question_id: question.question_id,
        title: question.title,
        description: question.description,
        required: question.required,
        single_select: question.single_select,
        options: question
            .options
            .into_iter()
            .map(protocol_option_to_governance)
            .collect(),
    }
}

fn protocol_option_to_governance(
    option: rekindle_protocol::dht::community::onboarding::OnboardingOption,
) -> rekindle_types::governance::OnboardingOption {
    rekindle_types::governance::OnboardingOption {
        option_id: option.option_id,
        title: option.title,
        description: option.description,
        emoji: None,
        roles_to_assign: option
            .roles_to_assign
            .into_iter()
            .map(u32_to_role_id)
            .collect(),
        channels_to_show: option
            .channels_to_show
            .into_iter()
            .map(|id| rekindle_types::id::ChannelId(hex_to_id_16(&id)))
            .collect(),
    }
}

pub fn protocol_guide_step_to_governance(
    step: rekindle_protocol::dht::community::onboarding::GuideStep,
) -> rekindle_types::governance::GuideStep {
    rekindle_types::governance::GuideStep {
        title: step.title,
        description: step.description,
        channel_id: step
            .channel_id
            .as_deref()
            .map(hex_to_id_16)
            .map(rekindle_types::id::ChannelId),
        emoji: step.emoji,
    }
}

pub fn protocol_welcome_channel_to_governance(
    channel: rekindle_protocol::dht::community::onboarding::WelcomeChannelEntry,
) -> rekindle_types::governance::WelcomeChannel {
    rekindle_types::governance::WelcomeChannel {
        channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel.channel_id)),
        description: channel.description,
        emoji: channel.emoji,
    }
}

fn governance_question_to_protocol(
    question: &rekindle_types::governance::OnboardingQuestion,
) -> rekindle_protocol::dht::community::onboarding::OnboardingQuestion {
    rekindle_protocol::dht::community::onboarding::OnboardingQuestion {
        question_id: question.question_id.clone(),
        title: question.title.clone(),
        description: question.description.clone(),
        required: question.required,
        single_select: question.single_select,
        options: question
            .options
            .iter()
            .map(governance_option_to_protocol)
            .collect(),
    }
}

fn governance_option_to_protocol(
    option: &rekindle_types::governance::OnboardingOption,
) -> rekindle_protocol::dht::community::onboarding::OnboardingOption {
    rekindle_protocol::dht::community::onboarding::OnboardingOption {
        option_id: option.option_id.clone(),
        title: option.title.clone(),
        description: option.description.clone(),
        roles_to_assign: option
            .roles_to_assign
            .iter()
            .map(|rid| u32::from_le_bytes([rid.0[0], rid.0[1], rid.0[2], rid.0[3]]))
            .collect(),
        channels_to_show: option
            .channels_to_show
            .iter()
            .map(|id| hex::encode(id.0))
            .collect(),
    }
}

fn governance_guide_step_to_protocol(
    step: &rekindle_types::governance::GuideStep,
) -> rekindle_protocol::dht::community::onboarding::GuideStep {
    rekindle_protocol::dht::community::onboarding::GuideStep {
        title: step.title.clone(),
        description: step.description.clone(),
        channel_id: step.channel_id.as_ref().map(|id| hex::encode(id.0)),
        emoji: step.emoji.clone(),
    }
}
