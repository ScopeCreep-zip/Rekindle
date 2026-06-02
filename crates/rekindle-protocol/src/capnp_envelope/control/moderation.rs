//! `control` moderation variant encode/decode helpers.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_envelope_capnp as cap;
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;

pub(super) fn write_kick(mut p: cap::kick_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::Kick { target_pseudonym } = payload else {
        unreachable!("write_kick: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

pub(super) fn write_ban(mut p: cap::ban_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::Ban { target_pseudonym } = payload else {
        unreachable!("write_ban: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

pub(super) fn write_unban(mut p: cap::unban_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::Unban { target_pseudonym } = payload else {
        unreachable!("write_unban: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

pub(super) fn write_timeout_member(
    mut p: cap::timeout_member_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::TimeoutMember {
        target_pseudonym,
        duration_seconds,
        reason,
    } = payload
    else {
        unreachable!("write_timeout_member: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
    p.set_duration_seconds(*duration_seconds);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
}

pub(super) fn write_remove_timeout(
    mut p: cap::remove_timeout_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::RemoveTimeout { target_pseudonym } = payload else {
        unreachable!("write_remove_timeout: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

pub(super) fn write_member_timed_out(
    mut p: cap::member_timed_out_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MemberTimedOut {
        pseudonym_key,
        timeout_until,
    } = payload
    else {
        unreachable!("write_member_timed_out: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
    p.set_has_timeout_until(timeout_until.is_some());
    if let Some(t) = timeout_until {
        p.set_timeout_until(*t);
    }
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

pub(super) fn read_kick(p: cap::kick_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::Kick {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_ban(p: cap::ban_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::Ban {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_unban(
    p: cap::unban_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::Unban {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_timeout_member(
    p: cap::timeout_member_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::TimeoutMember {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
        duration_seconds: p.get_duration_seconds(),
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

pub(super) fn read_remove_timeout(
    p: cap::remove_timeout_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::RemoveTimeout {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_member_timed_out(
    p: cap::member_timed_out_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MemberTimedOut {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        timeout_until: if p.get_has_timeout_until() {
            Some(p.get_timeout_until())
        } else {
            None
        },
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
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SystemMessage {
        body: text_to_string(p.get_body().map_err(|e| capnp_err(&e))?)?,
        timestamp: p.get_timestamp(),
    })
}
