//! `control` moderation variant encode/decode helpers.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community::envelope::ControlPayload;
use crate::community_envelope_capnp as cap;
use crate::error::CodecError;

pub(super) fn write_kick(mut p: cap::kick_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::Kick { target_pseudonym } = payload else {
        unreachable!("write_kick: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

pub(super) fn write_raid_alert(
    mut p: cap::raid_alert_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::RaidAlert { active } = payload else {
        unreachable!("write_raid_alert: variant mismatch")
    };
    p.set_active(*active);
}

pub(super) fn write_channel_lockdown(
    mut p: cap::channel_lockdown_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::ChannelLockdown { locked } = payload else {
        unreachable!("write_channel_lockdown: variant mismatch")
    };
    p.set_locked(*locked);
}

pub(super) fn write_system_message(
    mut p: cap::system_message_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::SystemMessage { body, timestamp } = payload else {
        unreachable!("write_system_message: variant mismatch")
    };
    p.set_body(body);
    p.set_timestamp(*timestamp);
}

pub(super) fn read_kick(p: cap::kick_payload::Reader<'_>) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::Kick {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_raid_alert(p: cap::raid_alert_payload::Reader<'_>) -> ControlPayload {
    ControlPayload::RaidAlert {
        active: p.get_active(),
    }
}

pub(super) fn read_channel_lockdown(
    p: cap::channel_lockdown_payload::Reader<'_>,
) -> ControlPayload {
    ControlPayload::ChannelLockdown {
        locked: p.get_locked(),
    }
}

pub(super) fn read_system_message(
    p: cap::system_message_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::SystemMessage {
        body: text_to_string(p.get_body().map_err(|e| capnp_err(&e))?)?,
        timestamp: p.get_timestamp(),
    })
}
