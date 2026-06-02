//! `governance` onboarding variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{channel_id_from_capnp, uuid16_to_capnp};
use super::shared::{
    read_guide_step, read_onboarding_question, read_welcome_channel, write_guide_step,
    write_onboarding_question, write_welcome_channel,
};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::ProtocolError;
use rekindle_types::governance::{GovernanceEntry, GuideStep, OnboardingQuestion, WelcomeChannel};
use rekindle_types::id::ChannelId;

pub(super) fn write_onboarding_config(
    mut p: schema_pkg::onboarding_config_entry::Builder<'_>,
    enabled: bool,
    mode: &str,
    default_channels: &[ChannelId],
    questions: &[OnboardingQuestion],
    welcome_message: Option<&str>,
    guide_steps: &[GuideStep],
    lamport: u64,
) {
    p.set_enabled(enabled);
    p.set_mode(mode);
    let mut chans = p
        .reborrow()
        .init_default_channels(len_u32(default_channels.len()));
    for (i, c) in default_channels.iter().enumerate() {
        uuid16_to_capnp(chans.reborrow().get(len_u32(i)), &c.0);
    }
    let mut q_list = p.reborrow().init_questions(len_u32(questions.len()));
    for (i, q) in questions.iter().enumerate() {
        write_onboarding_question(q_list.reborrow().get(len_u32(i)), q);
    }
    p.set_has_welcome_message(welcome_message.is_some());
    if let Some(m) = welcome_message {
        p.set_welcome_message(m);
    }
    let mut steps = p.reborrow().init_guide_steps(len_u32(guide_steps.len()));
    for (i, s) in guide_steps.iter().enumerate() {
        write_guide_step(steps.reborrow().get(len_u32(i)), s);
    }
    p.set_lamport(lamport);
}

pub(super) fn write_welcome_screen(
    mut p: schema_pkg::welcome_screen_entry::Builder<'_>,
    description: &str,
    channels: &[WelcomeChannel],
    lamport: u64,
) {
    p.set_description(description);
    let mut list = p.reborrow().init_channels(len_u32(channels.len()));
    for (i, c) in channels.iter().enumerate() {
        write_welcome_channel(list.reborrow().get(len_u32(i)), c);
    }
    p.set_lamport(lamport);
}

pub(super) fn read_onboarding_config(
    p: schema_pkg::onboarding_config_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let default_channels: Result<Vec<ChannelId>, ProtocolError> = p
        .get_default_channels()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(channel_id_from_capnp)
        .collect();
    let questions: Result<Vec<OnboardingQuestion>, ProtocolError> = p
        .get_questions()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_onboarding_question)
        .collect();
    let guide_steps: Result<Vec<GuideStep>, ProtocolError> = p
        .get_guide_steps()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_guide_step)
        .collect();
    Ok(GovernanceEntry::OnboardingConfig {
        enabled: p.get_enabled(),
        mode: text_to_string(p.get_mode().map_err(|e| capnp_err(&e))?)?,
        default_channels: default_channels?,
        questions: questions?,
        welcome_message: if p.get_has_welcome_message() {
            Some(text_to_string(
                p.get_welcome_message().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        guide_steps: guide_steps?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_welcome_screen(
    p: schema_pkg::welcome_screen_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let channels: Result<Vec<WelcomeChannel>, ProtocolError> = p
        .get_channels()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_welcome_channel)
        .collect();
    Ok(GovernanceEntry::WelcomeScreen {
        description: text_to_string(p.get_description().map_err(|e| capnp_err(&e))?)?,
        channels: channels?,
        lamport: p.get_lamport(),
    })
}
