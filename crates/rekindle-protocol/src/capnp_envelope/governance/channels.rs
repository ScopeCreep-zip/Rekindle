//! `governance` channels variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{category_id_from_capnp, channel_id_from_capnp, uuid16_to_capnp};
use super::shared::CategoryUpdate;
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::ProtocolError;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{CategoryId, ChannelId};

pub(super) fn write_channel_created(
    mut p: schema_pkg::channel_created_entry::Builder<'_>,
    e: &GovernanceEntry,
) {
    let GovernanceEntry::ChannelCreated {
        channel_id,
        name,
        channel_type,
        record_key,
        category_id,
        position,
        parent_voice_channel_id,
        lamport,
    } = e
    else {
        unreachable!("write_channel_created: variant mismatch")
    };
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_name(name);
    p.set_channel_type(channel_type);
    p.set_record_key(record_key);
    p.set_has_category_id(category_id.is_some());
    if let Some(c) = category_id {
        uuid16_to_capnp(p.reborrow().init_category_id(), &c.0);
    }
    p.set_position(*position);
    p.set_has_parent_voice_channel_id(parent_voice_channel_id.is_some());
    if let Some(pv) = parent_voice_channel_id {
        uuid16_to_capnp(p.reborrow().init_parent_voice_channel_id(), &pv.0);
    }
    p.set_lamport(*lamport);
}

pub(super) fn write_channel_archived(
    mut p: schema_pkg::channel_archived_entry::Builder<'_>,
    channel_id: ChannelId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_lamport(lamport);
}

pub(super) fn write_channel_updated(
    mut p: schema_pkg::channel_updated_entry::Builder<'_>,
    channel_id: ChannelId,
    name: Option<&str>,
    topic: Option<&str>,
    forum_tags: Option<&[String]>,
    position: Option<u32>,
    slowmode_seconds: Option<u32>,
    nsfw: Option<bool>,
    category_id: CategoryUpdate,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_has_name(name.is_some());
    if let Some(n) = name {
        p.set_name(n);
    }
    p.set_has_topic(topic.is_some());
    if let Some(t) = topic {
        p.set_topic(t);
    }
    p.set_has_forum_tags(forum_tags.is_some());
    if let Some(tags) = forum_tags {
        let mut list = p.reborrow().init_forum_tags(len_u32(tags.len()));
        for (i, t) in tags.iter().enumerate() {
            list.set(len_u32(i), t.as_str());
        }
    }
    p.set_has_position(position.is_some());
    if let Some(pos) = position {
        p.set_position(pos);
    }
    p.set_has_slowmode_seconds(slowmode_seconds.is_some());
    if let Some(s) = slowmode_seconds {
        p.set_slowmode_seconds(s);
    }
    p.set_has_nsfw(nsfw.is_some());
    if let Some(n) = nsfw {
        p.set_nsfw(n);
    }
    match category_id {
        CategoryUpdate::Unchanged => {
            p.set_has_category_id(false);
        }
        CategoryUpdate::Cleared => {
            p.set_has_category_id(true);
            p.set_category_id_present(false);
        }
        CategoryUpdate::Set(c) => {
            p.set_has_category_id(true);
            p.set_category_id_present(true);
            uuid16_to_capnp(p.reborrow().init_category_id(), &c.0);
        }
    }
    p.set_lamport(lamport);
}

pub(super) fn write_category_created(
    mut p: schema_pkg::category_created_entry::Builder<'_>,
    category_id: CategoryId,
    name: &str,
    position: u32,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_category_id(), &category_id.0);
    p.set_name(name);
    p.set_position(position);
    p.set_lamport(lamport);
}

pub(super) fn write_category_archived(
    mut p: schema_pkg::category_archived_entry::Builder<'_>,
    category_id: CategoryId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_category_id(), &category_id.0);
    p.set_lamport(lamport);
}

pub(super) fn write_permission_overwrite(
    mut p: schema_pkg::permission_overwrite_entry::Builder<'_>,
    channel_id: ChannelId,
    target_type: &str,
    target_id: &str,
    allow: u64,
    deny: u64,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_target_type(target_type);
    p.set_target_id(target_id);
    p.set_allow(allow);
    p.set_deny(deny);
    p.set_lamport(lamport);
}

pub(super) fn write_channel_segment_linked(
    mut p: schema_pkg::channel_segment_linked_entry::Builder<'_>,
    channel_id: ChannelId,
    segment_index: u32,
    record_key: &str,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_segment_index(segment_index);
    p.set_record_key(record_key);
    p.set_lamport(lamport);
}

pub(super) fn write_category_updated(
    mut p: schema_pkg::category_updated_entry::Builder<'_>,
    category_id: CategoryId,
    name: Option<&str>,
    position: Option<u32>,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_category_id(), &category_id.0);
    p.set_has_name(name.is_some());
    if let Some(n) = name {
        p.set_name(n);
    }
    p.set_has_position(position.is_some());
    if let Some(pos) = position {
        p.set_position(pos);
    }
    p.set_lamport(lamport);
}

pub(super) fn read_channel_created(
    p: schema_pkg::channel_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ChannelCreated {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        channel_type: text_to_string(p.get_channel_type().map_err(|e| capnp_err(&e))?)?,
        record_key: text_to_string(p.get_record_key().map_err(|e| capnp_err(&e))?)?,
        category_id: if p.get_has_category_id() {
            Some(category_id_from_capnp(
                p.get_category_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        position: p.get_position(),
        parent_voice_channel_id: if p.get_has_parent_voice_channel_id() {
            Some(channel_id_from_capnp(
                p.get_parent_voice_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_channel_archived(
    p: schema_pkg::channel_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ChannelArchived {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_channel_updated(
    p: schema_pkg::channel_updated_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let forum_tags = if p.get_has_forum_tags() {
        let list = p.get_forum_tags().map_err(|e| capnp_err(&e))?;
        let v: Result<Vec<String>, ProtocolError> = list
            .iter()
            .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
            .collect();
        Some(v?)
    } else {
        None
    };
    let category_id: Option<Option<CategoryId>> = if p.get_has_category_id() {
        if p.get_category_id_present() {
            Some(Some(category_id_from_capnp(
                p.get_category_id().map_err(|e| capnp_err(&e))?,
            )?))
        } else {
            Some(None)
        }
    } else {
        None
    };
    Ok(GovernanceEntry::ChannelUpdated {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        name: if p.get_has_name() {
            Some(text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        topic: if p.get_has_topic() {
            Some(text_to_string(p.get_topic().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        forum_tags,
        position: if p.get_has_position() {
            Some(p.get_position())
        } else {
            None
        },
        slowmode_seconds: if p.get_has_slowmode_seconds() {
            Some(p.get_slowmode_seconds())
        } else {
            None
        },
        nsfw: if p.get_has_nsfw() {
            Some(p.get_nsfw())
        } else {
            None
        },
        category_id,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_category_created(
    p: schema_pkg::category_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CategoryCreated {
        category_id: category_id_from_capnp(p.get_category_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        position: p.get_position(),
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_category_archived(
    p: schema_pkg::category_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CategoryArchived {
        category_id: category_id_from_capnp(p.get_category_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_permission_overwrite(
    p: schema_pkg::permission_overwrite_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::PermissionOverwrite {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        target_type: text_to_string(p.get_target_type().map_err(|e| capnp_err(&e))?)?,
        target_id: text_to_string(p.get_target_id().map_err(|e| capnp_err(&e))?)?,
        allow: p.get_allow(),
        deny: p.get_deny(),
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_channel_segment_linked(
    p: schema_pkg::channel_segment_linked_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ChannelSegmentLinked {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        segment_index: p.get_segment_index(),
        record_key: text_to_string(p.get_record_key().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_category_updated(
    p: schema_pkg::category_updated_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CategoryUpdated {
        category_id: category_id_from_capnp(p.get_category_id().map_err(|e| capnp_err(&e))?)?,
        name: if p.get_has_name() {
            Some(text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        position: if p.get_has_position() {
            Some(p.get_position())
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}
