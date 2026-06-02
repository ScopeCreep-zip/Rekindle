//! `control` events variant encode/decode helpers.

use super::super::sub_types::{
    read_event_info, read_game_server_info, read_thread_info, write_event_info,
    write_game_server_info, write_thread_info,
};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_envelope_capnp as cap;
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;

pub(super) fn write_event_created(
    p: cap::event_created_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::EventCreated { event } = payload else {
        unreachable!("write_event_created: variant mismatch")
    };
    write_event_info(p.init_event(), event);
}

pub(super) fn write_event_updated(
    p: cap::event_updated_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::EventUpdated { event } = payload else {
        unreachable!("write_event_updated: variant mismatch")
    };
    write_event_info(p.init_event(), event);
}

pub(super) fn write_event_deleted(
    mut p: cap::event_deleted_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::EventDeleted { event_id } = payload else {
        unreachable!("write_event_deleted: variant mismatch")
    };
    p.set_event_id(event_id);
}

pub(super) fn write_event_rsvp_changed(
    mut p: cap::event_rsvp_changed_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::EventRsvpChanged {
        event_id,
        pseudonym_key,
        status,
    } = payload
    else {
        unreachable!("write_event_rsvp_changed: variant mismatch")
    };
    p.set_event_id(event_id);
    p.set_pseudonym_key(pseudonym_key);
    p.set_status(status);
}

pub(super) fn write_thread_created(
    p: cap::thread_created_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::ThreadCreated { thread } = payload else {
        unreachable!("write_thread_created: variant mismatch")
    };
    write_thread_info(p.init_thread(), thread);
}

pub(super) fn write_thread_message_received(
    mut p: cap::thread_message_received_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::ThreadMessageReceived {
        thread_id,
        message_id,
        sender_pseudonym,
        ciphertext,
        mek_generation,
        timestamp,
        reply_to_id,
    } = payload
    else {
        unreachable!("write_thread_message_received: variant mismatch")
    };
    p.set_thread_id(thread_id);
    p.set_message_id(message_id);
    p.set_sender_pseudonym(sender_pseudonym);
    p.set_ciphertext(ciphertext);
    p.set_mek_generation(*mek_generation);
    p.set_timestamp(*timestamp);
    p.set_has_reply_to_id(reply_to_id.is_some());
    if let Some(r) = reply_to_id {
        p.set_reply_to_id(r);
    }
}

pub(super) fn write_thread_archived(
    mut p: cap::thread_archived_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::ThreadArchived {
        thread_id,
        archived,
    } = payload
    else {
        unreachable!("write_thread_archived: variant mismatch")
    };
    p.set_thread_id(thread_id);
    p.set_archived(*archived);
}

pub(super) fn write_game_server_added(
    p: cap::game_server_added_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::GameServerAdded { server } = payload else {
        unreachable!("write_game_server_added: variant mismatch")
    };
    write_game_server_info(p.init_server(), server);
}

pub(super) fn write_game_server_removed(
    mut p: cap::game_server_removed_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::GameServerRemoved { server_id } = payload else {
        unreachable!("write_game_server_removed: variant mismatch")
    };
    p.set_server_id(server_id);
}

pub(super) fn write_event_reminder(
    mut p: cap::event_reminder_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::EventReminder {
        event_id,
        title,
        minutes_until_start,
    } = payload
    else {
        unreachable!("write_event_reminder: variant mismatch")
    };
    p.set_event_id(event_id);
    p.set_title(title);
    p.set_minutes_until_start(*minutes_until_start);
}

pub(super) fn read_event_created(
    p: cap::event_created_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventCreated {
        event: read_event_info(p.get_event().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_event_updated(
    p: cap::event_updated_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventUpdated {
        event: read_event_info(p.get_event().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_event_deleted(
    p: cap::event_deleted_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventDeleted {
        event_id: text_to_string(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_event_rsvp_changed(
    p: cap::event_rsvp_changed_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventRsvpChanged {
        event_id: text_to_string(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        status: text_to_string(p.get_status().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_thread_created(
    p: cap::thread_created_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ThreadCreated {
        thread: read_thread_info(p.get_thread().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_thread_message_received(
    p: cap::thread_message_received_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ThreadMessageReceived {
        thread_id: text_to_string(p.get_thread_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        sender_pseudonym: text_to_string(p.get_sender_pseudonym().map_err(|e| capnp_err(&e))?)?,
        ciphertext: p.get_ciphertext().map_err(|e| capnp_err(&e))?.to_vec(),
        mek_generation: p.get_mek_generation(),
        timestamp: p.get_timestamp(),
        reply_to_id: if p.get_has_reply_to_id() {
            Some(text_to_string(
                p.get_reply_to_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
    })
}

pub(super) fn read_thread_archived(
    p: cap::thread_archived_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ThreadArchived {
        thread_id: text_to_string(p.get_thread_id().map_err(|e| capnp_err(&e))?)?,
        archived: p.get_archived(),
    })
}

pub(super) fn read_game_server_added(
    p: cap::game_server_added_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::GameServerAdded {
        server: read_game_server_info(p.get_server().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_game_server_removed(
    p: cap::game_server_removed_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::GameServerRemoved {
        server_id: text_to_string(p.get_server_id().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_event_reminder(
    p: cap::event_reminder_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventReminder {
        event_id: text_to_string(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
        title: text_to_string(p.get_title().map_err(|e| capnp_err(&e))?)?,
        minutes_until_start: p.get_minutes_until_start(),
    })
}
