//! `governance` shared sub-type encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{channel_id_from_capnp, role_id_from_capnp, uuid16_to_capnp};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_event_capnp;
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::ProtocolError;
use rekindle_types::expression::SoundboardMeta;
use rekindle_types::governance::{GuideStep, OnboardingOption, OnboardingQuestion, WelcomeChannel};
use rekindle_types::id::{CategoryId, ChannelId, RoleId};

/// Tri-state for `ChannelUpdated.category_id`. The Rust enum variant
/// uses `Option<Option<CategoryId>>` (clippy `option_option` smells)
/// to distinguish "no change" / "clear category" / "set category", so
/// the encoder converts to this explicit shape at the dispatcher
/// boundary. Conversion is inlined where used to avoid lifting the
/// `Option<Option<...>>` shape into any helper signature.
#[derive(Clone, Copy)]
pub(super) enum CategoryUpdate {
    Unchanged,
    Cleared,
    Set(CategoryId),
}

pub(super) fn event_status_to_capnp(
    s: rekindle_types::event::EventStatus,
) -> community_event_capnp::EventStatus {
    use community_event_capnp::EventStatus as Cap;
    use rekindle_types::event::EventStatus;
    match s {
        EventStatus::Scheduled => Cap::Scheduled,
        EventStatus::Active => Cap::Active,
        EventStatus::Completed => Cap::Completed,
        EventStatus::Cancelled => Cap::Cancelled,
    }
}

pub(super) fn event_status_from_capnp(
    s: community_event_capnp::EventStatus,
) -> rekindle_types::event::EventStatus {
    use community_event_capnp::EventStatus as Cap;
    use rekindle_types::event::EventStatus;
    match s {
        Cap::Scheduled => EventStatus::Scheduled,
        Cap::Active => EventStatus::Active,
        Cap::Completed => EventStatus::Completed,
        Cap::Cancelled => EventStatus::Cancelled,
    }
}

pub(super) fn write_sound_meta(
    mut b: schema_pkg::soundboard_meta::Builder<'_>,
    s: &SoundboardMeta,
) {
    b.set_duration_seconds(s.duration_seconds);
    b.set_volume(s.volume);
    b.set_has_emoji(s.emoji.is_some());
    if let Some(ref e) = s.emoji {
        b.set_emoji(e);
    }
}

pub(super) fn read_sound_meta(
    r: schema_pkg::soundboard_meta::Reader<'_>,
) -> Result<SoundboardMeta, ProtocolError> {
    Ok(SoundboardMeta {
        duration_seconds: r.get_duration_seconds(),
        volume: r.get_volume(),
        emoji: if r.get_has_emoji() {
            Some(text_to_string(r.get_emoji().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

pub(super) fn write_onboarding_question(
    mut b: schema_pkg::onboarding_question::Builder<'_>,
    q: &OnboardingQuestion,
) {
    b.set_question_id(&q.question_id);
    b.set_title(&q.title);
    b.set_has_description(q.description.is_some());
    if let Some(ref d) = q.description {
        b.set_description(d);
    }
    b.set_required(q.required);
    b.set_single_select(q.single_select);
    let mut opts = b.reborrow().init_options(len_u32(q.options.len()));
    for (i, o) in q.options.iter().enumerate() {
        write_onboarding_option(opts.reborrow().get(len_u32(i)), o);
    }
}

pub(super) fn read_onboarding_question(
    r: schema_pkg::onboarding_question::Reader<'_>,
) -> Result<OnboardingQuestion, ProtocolError> {
    let options: Result<Vec<OnboardingOption>, ProtocolError> = r
        .get_options()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_onboarding_option)
        .collect();
    Ok(OnboardingQuestion {
        question_id: text_to_string(r.get_question_id().map_err(|e| capnp_err(&e))?)?,
        title: text_to_string(r.get_title().map_err(|e| capnp_err(&e))?)?,
        description: if r.get_has_description() {
            Some(text_to_string(
                r.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        required: r.get_required(),
        single_select: r.get_single_select(),
        options: options?,
    })
}

pub(super) fn write_onboarding_option(
    mut b: schema_pkg::onboarding_option::Builder<'_>,
    o: &OnboardingOption,
) {
    b.set_option_id(&o.option_id);
    b.set_title(&o.title);
    b.set_has_description(o.description.is_some());
    if let Some(ref d) = o.description {
        b.set_description(d);
    }
    b.set_has_emoji(o.emoji.is_some());
    if let Some(ref e) = o.emoji {
        b.set_emoji(e);
    }
    let mut roles = b
        .reborrow()
        .init_roles_to_assign(len_u32(o.roles_to_assign.len()));
    for (i, r) in o.roles_to_assign.iter().enumerate() {
        uuid16_to_capnp(roles.reborrow().get(len_u32(i)), &r.0);
    }
    let mut chans = b
        .reborrow()
        .init_channels_to_show(len_u32(o.channels_to_show.len()));
    for (i, c) in o.channels_to_show.iter().enumerate() {
        uuid16_to_capnp(chans.reborrow().get(len_u32(i)), &c.0);
    }
}

pub(super) fn read_onboarding_option(
    r: schema_pkg::onboarding_option::Reader<'_>,
) -> Result<OnboardingOption, ProtocolError> {
    let roles_to_assign: Result<Vec<RoleId>, ProtocolError> = r
        .get_roles_to_assign()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(role_id_from_capnp)
        .collect();
    let channels_to_show: Result<Vec<ChannelId>, ProtocolError> = r
        .get_channels_to_show()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(channel_id_from_capnp)
        .collect();
    Ok(OnboardingOption {
        option_id: text_to_string(r.get_option_id().map_err(|e| capnp_err(&e))?)?,
        title: text_to_string(r.get_title().map_err(|e| capnp_err(&e))?)?,
        description: if r.get_has_description() {
            Some(text_to_string(
                r.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        emoji: if r.get_has_emoji() {
            Some(text_to_string(r.get_emoji().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        roles_to_assign: roles_to_assign?,
        channels_to_show: channels_to_show?,
    })
}

pub(super) fn write_guide_step(mut b: schema_pkg::guide_step::Builder<'_>, s: &GuideStep) {
    b.set_title(&s.title);
    b.set_description(&s.description);
    b.set_has_channel_id(s.channel_id.is_some());
    if let Some(ref c) = s.channel_id {
        uuid16_to_capnp(b.reborrow().init_channel_id(), &c.0);
    }
    b.set_has_emoji(s.emoji.is_some());
    if let Some(ref e) = s.emoji {
        b.set_emoji(e);
    }
}

pub(super) fn read_guide_step(
    r: schema_pkg::guide_step::Reader<'_>,
) -> Result<GuideStep, ProtocolError> {
    Ok(GuideStep {
        title: text_to_string(r.get_title().map_err(|e| capnp_err(&e))?)?,
        description: text_to_string(r.get_description().map_err(|e| capnp_err(&e))?)?,
        channel_id: if r.get_has_channel_id() {
            Some(channel_id_from_capnp(
                r.get_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        emoji: if r.get_has_emoji() {
            Some(text_to_string(r.get_emoji().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

pub(super) fn write_welcome_channel(
    mut b: schema_pkg::welcome_channel::Builder<'_>,
    w: &WelcomeChannel,
) {
    uuid16_to_capnp(b.reborrow().init_channel_id(), &w.channel_id.0);
    b.set_description(&w.description);
    b.set_has_emoji(w.emoji.is_some());
    if let Some(ref e) = w.emoji {
        b.set_emoji(e);
    }
}

pub(super) fn read_welcome_channel(
    r: schema_pkg::welcome_channel::Reader<'_>,
) -> Result<WelcomeChannel, ProtocolError> {
    Ok(WelcomeChannel {
        channel_id: channel_id_from_capnp(r.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        description: text_to_string(r.get_description().map_err(|e| capnp_err(&e))?)?,
        emoji: if r.get_has_emoji() {
            Some(text_to_string(r.get_emoji().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}
