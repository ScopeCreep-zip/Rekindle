//! `governance` events variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{
    channel_id_from_capnp, event_id_from_capnp, pseudonym_key_from_capnp, pseudonym_key_to_capnp,
    read_event_location_via_event_capnp, read_recurrence_rule_via_event_capnp,
    thread_id_from_capnp, uuid16_to_capnp, write_event_location_via_event_capnp,
    write_recurrence_rule_via_event_capnp,
};
use super::shared::{event_status_from_capnp, event_status_to_capnp};
use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::ProtocolError;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, EventId, PseudonymKey, ThreadId};

pub(super) fn write_thread_created(
    mut p: schema_pkg::thread_created_entry::Builder<'_>,
    thread_id: ThreadId,
    parent_channel_id: ChannelId,
    name: &str,
    thread_type: &str,
    record_key: Option<&str>,
    invited: &[PseudonymKey],
    forum_tag: Option<&str>,
    auto_archive_seconds: u64,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_thread_id(), &thread_id.0);
    uuid16_to_capnp(p.reborrow().init_parent_channel_id(), &parent_channel_id.0);
    p.set_name(name);
    p.set_thread_type(thread_type);
    p.set_has_record_key(record_key.is_some());
    if let Some(k) = record_key {
        p.set_record_key(k);
    }
    let mut inv_list = p.reborrow().init_invited(len_u32(invited.len()));
    for (i, k) in invited.iter().enumerate() {
        pseudonym_key_to_capnp(inv_list.reborrow().get(len_u32(i)), k);
    }
    p.set_has_forum_tag(forum_tag.is_some());
    if let Some(t) = forum_tag {
        p.set_forum_tag(t);
    }
    p.set_auto_archive_seconds(auto_archive_seconds);
    p.set_lamport(lamport);
}

pub(super) fn write_thread_archived(
    mut p: schema_pkg::thread_archived_entry::Builder<'_>,
    thread_id: ThreadId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_thread_id(), &thread_id.0);
    p.set_lamport(lamport);
}

// `write_event_created` and `write_expression_added` re-extract their
// fields via `let ... else { unreachable!() }` so the helper signature
// stays under the `too_many_arguments` threshold without needing
// `#[allow]`. Matches the per-variant pattern in `control.rs`.
pub(super) fn write_event_created(
    mut p: schema_pkg::event_created_entry::Builder<'_>,
    e: &GovernanceEntry,
) {
    let GovernanceEntry::EventCreated {
        event_id,
        name,
        description,
        start_time,
        end_time,
        channel_id,
        cover_image_ref,
        creator_pseudonym,
        recurrence,
        location,
        status,
        lamport,
    } = e
    else {
        unreachable!("write_event_created: variant mismatch")
    };
    uuid16_to_capnp(p.reborrow().init_event_id(), &event_id.0);
    p.set_name(name);
    p.set_has_description(description.is_some());
    if let Some(d) = description {
        p.set_description(d);
    }
    p.set_start_time(*start_time);
    p.set_has_end_time(end_time.is_some());
    if let Some(t) = end_time {
        p.set_end_time(*t);
    }
    p.set_has_channel_id(channel_id.is_some());
    if let Some(c) = channel_id {
        uuid16_to_capnp(p.reborrow().init_channel_id(), &c.0);
    }
    p.set_has_cover_image_ref(cover_image_ref.is_some());
    if let Some(r) = cover_image_ref {
        p.set_cover_image_ref(r);
    }
    p.set_has_creator_pseudonym(creator_pseudonym.is_some());
    if let Some(c) = creator_pseudonym {
        pseudonym_key_to_capnp(p.reborrow().init_creator_pseudonym(), c);
    }
    p.set_has_recurrence(recurrence.is_some());
    if let Some(r) = recurrence {
        write_recurrence_rule_via_event_capnp(p.reborrow().init_recurrence(), r);
    }
    p.set_has_location(location.is_some());
    if let Some(loc) = location {
        write_event_location_via_event_capnp(p.reborrow().init_location(), loc);
    }
    p.set_has_status(status.is_some());
    if let Some(s) = status {
        p.set_status(event_status_to_capnp(*s));
    }
    p.set_lamport(*lamport);
}

pub(super) fn write_event_archived(
    mut p: schema_pkg::event_archived_entry::Builder<'_>,
    event_id: EventId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_event_id(), &event_id.0);
    p.set_lamport(lamport);
}

pub(super) fn read_thread_created(
    p: schema_pkg::thread_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let invited: Result<Vec<_>, ProtocolError> = p
        .get_invited()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(pseudonym_key_from_capnp)
        .collect();
    Ok(GovernanceEntry::ThreadCreated {
        thread_id: thread_id_from_capnp(p.get_thread_id().map_err(|e| capnp_err(&e))?)?,
        parent_channel_id: channel_id_from_capnp(
            p.get_parent_channel_id().map_err(|e| capnp_err(&e))?,
        )?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        thread_type: text_to_string(p.get_thread_type().map_err(|e| capnp_err(&e))?)?,
        record_key: if p.get_has_record_key() {
            Some(text_to_string(
                p.get_record_key().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        invited: invited?,
        forum_tag: if p.get_has_forum_tag() {
            Some(text_to_string(
                p.get_forum_tag().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        auto_archive_seconds: p.get_auto_archive_seconds(),
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_thread_archived(
    p: schema_pkg::thread_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ThreadArchived {
        thread_id: thread_id_from_capnp(p.get_thread_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_event_created(
    p: schema_pkg::event_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::EventCreated {
        event_id: event_id_from_capnp(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        description: if p.get_has_description() {
            Some(text_to_string(
                p.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        start_time: p.get_start_time(),
        end_time: if p.get_has_end_time() {
            Some(p.get_end_time())
        } else {
            None
        },
        channel_id: if p.get_has_channel_id() {
            Some(channel_id_from_capnp(
                p.get_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        cover_image_ref: if p.get_has_cover_image_ref() {
            Some(text_to_string(
                p.get_cover_image_ref().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        creator_pseudonym: if p.get_has_creator_pseudonym() {
            Some(pseudonym_key_from_capnp(
                p.get_creator_pseudonym().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        recurrence: if p.get_has_recurrence() {
            Some(read_recurrence_rule_via_event_capnp(
                p.get_recurrence().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        location: if p.get_has_location() {
            Some(read_event_location_via_event_capnp(
                p.get_location().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        status: if p.get_has_status() {
            Some(event_status_from_capnp(
                p.get_status().map_err(not_in_schema)?,
            ))
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_event_archived(
    p: schema_pkg::event_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::EventArchived {
        event_id: event_id_from_capnp(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}
