//! `control` messaging variant encode/decode helpers.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_envelope_capnp as cap;
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;

pub(super) fn write_message_edited(
    mut p: cap::message_edited_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MessageEdited {
        channel_id,
        message_id,
        new_ciphertext,
        mek_generation,
        edited_at,
    } = payload
    else {
        unreachable!("write_message_edited: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_new_ciphertext(new_ciphertext);
    p.set_mek_generation(*mek_generation);
    p.set_edited_at(*edited_at);
}

pub(super) fn write_message_deleted(
    mut p: cap::message_deleted_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MessageDeleted {
        channel_id,
        message_id,
    } = payload
    else {
        unreachable!("write_message_deleted: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
}

pub(super) fn write_reaction_added(
    mut p: cap::reaction_added_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::ReactionAdded {
        channel_id,
        message_id,
        emoji,
        reactor_pseudonym,
    } = payload
    else {
        unreachable!("write_reaction_added: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_emoji(emoji);
    p.set_reactor_pseudonym(reactor_pseudonym);
}

pub(super) fn write_reaction_removed(
    mut p: cap::reaction_removed_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::ReactionRemoved {
        channel_id,
        message_id,
        emoji,
        reactor_pseudonym,
    } = payload
    else {
        unreachable!("write_reaction_removed: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_emoji(emoji);
    p.set_reactor_pseudonym(reactor_pseudonym);
}

pub(super) fn write_message_pinned(
    mut p: cap::message_pinned_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MessagePinned {
        channel_id,
        message_id,
        pinned_by,
    } = payload
    else {
        unreachable!("write_message_pinned: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_pinned_by(pinned_by);
}

pub(super) fn write_message_unpinned(
    mut p: cap::message_unpinned_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MessageUnpinned {
        channel_id,
        message_id,
    } = payload
    else {
        unreachable!("write_message_unpinned: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
}

pub(super) fn write_link_preview(
    mut p: cap::link_preview_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::LinkPreview {
        channel_id,
        message_id,
        url,
        title,
        description,
        image_url,
        site_name,
        fetched_at,
    } = payload
    else {
        unreachable!("write_link_preview: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_url(url);
    p.set_has_title(title.is_some());
    if let Some(t) = title {
        p.set_title(t);
    }
    p.set_has_description(description.is_some());
    if let Some(d) = description {
        p.set_description(d);
    }
    p.set_has_image_url(image_url.is_some());
    if let Some(u) = image_url {
        p.set_image_url(u);
    }
    p.set_has_site_name(site_name.is_some());
    if let Some(s) = site_name {
        p.set_site_name(s);
    }
    p.set_fetched_at(*fetched_at);
}

pub(super) fn read_message_edited(
    p: cap::message_edited_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MessageEdited {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        new_ciphertext: p.get_new_ciphertext().map_err(|e| capnp_err(&e))?.to_vec(),
        mek_generation: p.get_mek_generation(),
        edited_at: p.get_edited_at(),
    })
}

pub(super) fn read_message_deleted(
    p: cap::message_deleted_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MessageDeleted {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_reaction_added(
    p: cap::reaction_added_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ReactionAdded {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        emoji: text_to_string(p.get_emoji().map_err(|e| capnp_err(&e))?)?,
        reactor_pseudonym: text_to_string(p.get_reactor_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_reaction_removed(
    p: cap::reaction_removed_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ReactionRemoved {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        emoji: text_to_string(p.get_emoji().map_err(|e| capnp_err(&e))?)?,
        reactor_pseudonym: text_to_string(p.get_reactor_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_message_pinned(
    p: cap::message_pinned_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MessagePinned {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        pinned_by: text_to_string(p.get_pinned_by().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_message_unpinned(
    p: cap::message_unpinned_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MessageUnpinned {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_link_preview(
    p: cap::link_preview_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::LinkPreview {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        url: text_to_string(p.get_url().map_err(|e| capnp_err(&e))?)?,
        title: if p.get_has_title() {
            Some(text_to_string(p.get_title().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        description: if p.get_has_description() {
            Some(text_to_string(
                p.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        image_url: if p.get_has_image_url() {
            Some(text_to_string(
                p.get_image_url().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        site_name: if p.get_has_site_name() {
            Some(text_to_string(
                p.get_site_name().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        fetched_at: p.get_fetched_at(),
    })
}
