//! Message-plane leaf codecs: presence game info, member info, thread
//! info, game servers, MEK delivery, bootstrap + synced messages.

use super::super::len_u32;
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::dht::community::envelope::PresenceGameInfo;
use crate::error::ProtocolError;
use rekindle_types::game_server::GameServerInfo;
use rekindle_types::mek::ChannelMekDelivery;
use rekindle_types::member::MemberInfo;
use rekindle_types::message::{BootstrapChannelMessages, BootstrapMessage, SyncedMessage};
use rekindle_types::thread::ThreadInfo;

// ── PresenceGameInfo (envelope.PresenceUpdate.game_info) ─────────────

pub(crate) fn write_presence_game_info(
    mut b: crate::community_envelope_capnp::presence_game_info::Builder<'_>,
    g: &PresenceGameInfo,
) {
    b.set_game_name(&g.game_name);
    b.set_has_game_id(g.game_id.is_some());
    if let Some(id) = g.game_id {
        b.set_game_id(id);
    }
    b.set_has_elapsed_secs(g.elapsed_seconds.is_some());
    if let Some(s) = g.elapsed_seconds {
        b.set_elapsed_seconds(s);
    }
    b.set_has_server_address(g.server_address.is_some());
    if let Some(ref addr) = g.server_address {
        b.set_server_address(addr);
    }
}

pub(crate) fn read_presence_game_info(
    r: crate::community_envelope_capnp::presence_game_info::Reader<'_>,
) -> Result<PresenceGameInfo, ProtocolError> {
    Ok(PresenceGameInfo {
        game_name: text_to_string(r.get_game_name().map_err(|e| capnp_err(&e))?)?,
        game_id: if r.get_has_game_id() {
            Some(r.get_game_id())
        } else {
            None
        },
        elapsed_seconds: if r.get_has_elapsed_secs() {
            Some(r.get_elapsed_seconds())
        } else {
            None
        },
        server_address: if r.get_has_server_address() {
            Some(text_to_string(
                r.get_server_address().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
    })
}

// ── MemberInfo ───────────────────────────────────────────────────────

pub(crate) fn write_member_info(
    mut b: crate::community_member_capnp::member_info::Builder<'_>,
    m: &MemberInfo,
) {
    b.set_pseudonym_key(&m.pseudonym_key);
    b.set_display_name(&m.display_name);
    let mut role_ids = b.reborrow().init_role_ids(len_u32(m.role_ids.len()));
    for (i, id) in m.role_ids.iter().enumerate() {
        role_ids.set(len_u32(i), *id);
    }
    b.set_status(&m.status);
    b.set_timeout_until(m.timeout_until.unwrap_or(0));
    if let Some(ref blob) = m.route_blob {
        b.set_route_blob(blob);
    }
    if let Some(ref bio) = m.bio {
        b.set_bio(bio);
    }
    if let Some(ref pr) = m.pronouns {
        b.set_pronouns(pr);
    }
    b.set_theme_color(m.theme_color.unwrap_or(0));
    let mut badges = b.reborrow().init_badges(len_u32(m.badges.len()));
    for (i, badge) in m.badges.iter().enumerate() {
        badges.set(len_u32(i), badge.as_str());
    }
    b.set_last_seen(m.last_seen);
}

pub(crate) fn read_member_info(
    r: crate::community_member_capnp::member_info::Reader<'_>,
) -> Result<MemberInfo, ProtocolError> {
    let role_ids: Vec<u32> = r
        .get_role_ids()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .collect();
    let route_blob_bytes = r.get_route_blob().map_err(|e| capnp_err(&e))?;
    let bio_text = text_to_string(r.get_bio().map_err(|e| capnp_err(&e))?)?;
    let pronouns_text = text_to_string(r.get_pronouns().map_err(|e| capnp_err(&e))?)?;
    let badges: Result<Vec<String>, ProtocolError> = r
        .get_badges()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect();
    let timeout = r.get_timeout_until();
    let theme = r.get_theme_color();
    Ok(MemberInfo {
        pseudonym_key: text_to_string(r.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        display_name: text_to_string(r.get_display_name().map_err(|e| capnp_err(&e))?)?,
        role_ids,
        status: text_to_string(r.get_status().map_err(|e| capnp_err(&e))?)?,
        timeout_until: if timeout == 0 { None } else { Some(timeout) },
        route_blob: if route_blob_bytes.is_empty() {
            None
        } else {
            Some(route_blob_bytes.to_vec())
        },
        bio: if bio_text.is_empty() {
            None
        } else {
            Some(bio_text)
        },
        pronouns: if pronouns_text.is_empty() {
            None
        } else {
            Some(pronouns_text)
        },
        theme_color: if theme == 0 { None } else { Some(theme) },
        badges: badges?,
        last_seen: r.get_last_seen(),
    })
}

// ── ThreadInfo ───────────────────────────────────────────────────────

pub(crate) fn write_thread_info(
    mut b: crate::community_thread_capnp::thread_info::Builder<'_>,
    t: &ThreadInfo,
) {
    b.set_id(&t.id);
    b.set_channel_id(&t.channel_id);
    b.set_name(&t.name);
    b.set_starter_message_id(&t.starter_message_id);
    b.set_creator_pseudonym(&t.creator_pseudonym);
    if let Some(ref tag) = t.forum_tag {
        b.set_forum_tag(tag);
    }
    b.set_created_at(t.created_at);
    b.set_archived(t.archived);
    b.set_auto_archive_seconds(t.auto_archive_seconds);
    b.set_last_message_at(t.last_message_at);
    b.set_message_count(t.message_count);
}

pub(crate) fn read_thread_info(
    r: crate::community_thread_capnp::thread_info::Reader<'_>,
) -> Result<ThreadInfo, ProtocolError> {
    let forum_tag_text = text_to_string(r.get_forum_tag().map_err(|e| capnp_err(&e))?)?;
    Ok(ThreadInfo {
        id: text_to_string(r.get_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: text_to_string(r.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(r.get_name().map_err(|e| capnp_err(&e))?)?,
        starter_message_id: text_to_string(r.get_starter_message_id().map_err(|e| capnp_err(&e))?)?,
        creator_pseudonym: text_to_string(r.get_creator_pseudonym().map_err(|e| capnp_err(&e))?)?,
        forum_tag: if forum_tag_text.is_empty() {
            None
        } else {
            Some(forum_tag_text)
        },
        created_at: r.get_created_at(),
        archived: r.get_archived(),
        auto_archive_seconds: r.get_auto_archive_seconds(),
        last_message_at: r.get_last_message_at(),
        message_count: r.get_message_count(),
    })
}

// ── GameServerInfo ───────────────────────────────────────────────────

pub(crate) fn write_game_server_info(
    mut b: crate::community_game_server_capnp::game_server_info::Builder<'_>,
    g: &GameServerInfo,
) {
    b.set_id(&g.id);
    b.set_game_id(&g.game_id);
    b.set_label(&g.label);
    b.set_address(&g.address);
    b.set_added_by(&g.added_by);
    b.set_created_at(g.created_at);
}

pub(crate) fn read_game_server_info(
    r: crate::community_game_server_capnp::game_server_info::Reader<'_>,
) -> Result<GameServerInfo, ProtocolError> {
    Ok(GameServerInfo {
        id: text_to_string(r.get_id().map_err(|e| capnp_err(&e))?)?,
        game_id: text_to_string(r.get_game_id().map_err(|e| capnp_err(&e))?)?,
        label: text_to_string(r.get_label().map_err(|e| capnp_err(&e))?)?,
        address: text_to_string(r.get_address().map_err(|e| capnp_err(&e))?)?,
        added_by: text_to_string(r.get_added_by().map_err(|e| capnp_err(&e))?)?,
        created_at: r.get_created_at(),
    })
}

// ── ChannelMekDelivery ───────────────────────────────────────────────

pub(crate) fn write_channel_mek_delivery(
    mut b: crate::community_mek_capnp::channel_mek_delivery::Builder<'_>,
    d: &ChannelMekDelivery,
) {
    if let Some(ref id) = d.channel_id {
        b.set_channel_id(id);
    }
    b.set_generation(d.generation);
    b.set_wrapped_mek(&d.wrapped_mek);
}

pub(crate) fn read_channel_mek_delivery(
    r: crate::community_mek_capnp::channel_mek_delivery::Reader<'_>,
) -> Result<ChannelMekDelivery, ProtocolError> {
    let channel_id_text = text_to_string(r.get_channel_id().map_err(|e| capnp_err(&e))?)?;
    Ok(ChannelMekDelivery {
        channel_id: if channel_id_text.is_empty() {
            None
        } else {
            Some(channel_id_text)
        },
        generation: r.get_generation(),
        wrapped_mek: r.get_wrapped_mek().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

// ── BootstrapMessage / BootstrapChannelMessages ──────────────────────

fn write_bootstrap_message(
    mut b: crate::community_message_capnp::bootstrap_message::Builder<'_>,
    m: &BootstrapMessage,
) {
    b.set_message_id(&m.message_id);
    b.set_sender_pseudonym(&m.sender_pseudonym);
    b.set_ciphertext(&m.ciphertext);
    b.set_mek_generation(m.mek_generation);
    b.set_timestamp(m.timestamp);
}

fn read_bootstrap_message(
    r: crate::community_message_capnp::bootstrap_message::Reader<'_>,
) -> Result<BootstrapMessage, ProtocolError> {
    Ok(BootstrapMessage {
        message_id: text_to_string(r.get_message_id().map_err(|e| capnp_err(&e))?)?,
        sender_pseudonym: text_to_string(r.get_sender_pseudonym().map_err(|e| capnp_err(&e))?)?,
        ciphertext: r.get_ciphertext().map_err(|e| capnp_err(&e))?.to_vec(),
        mek_generation: r.get_mek_generation(),
        timestamp: r.get_timestamp(),
    })
}

pub(crate) fn write_bootstrap_channel_messages(
    mut b: crate::community_message_capnp::bootstrap_channel_messages::Builder<'_>,
    g: &BootstrapChannelMessages,
) {
    b.set_channel_id(&g.channel_id);
    let mut list = b.reborrow().init_messages(len_u32(g.messages.len()));
    for (i, msg) in g.messages.iter().enumerate() {
        write_bootstrap_message(list.reborrow().get(len_u32(i)), msg);
    }
}

pub(crate) fn read_bootstrap_channel_messages(
    r: crate::community_message_capnp::bootstrap_channel_messages::Reader<'_>,
) -> Result<BootstrapChannelMessages, ProtocolError> {
    let messages: Result<Vec<BootstrapMessage>, ProtocolError> = r
        .get_messages()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_bootstrap_message)
        .collect();
    Ok(BootstrapChannelMessages {
        channel_id: text_to_string(r.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        messages: messages?,
    })
}

// ── SyncedMessage ────────────────────────────────────────────────────

pub(crate) fn write_synced_message(
    mut b: crate::community_message_capnp::synced_message::Builder<'_>,
    m: &SyncedMessage,
) {
    b.set_sender_key(&m.sender_key);
    b.set_body(&m.body);
    b.set_timestamp(m.timestamp);
    b.set_has_mek_generation(m.mek_generation.is_some());
    if let Some(g) = m.mek_generation {
        b.set_mek_generation(g);
    }
    b.set_has_lamport_ts(m.lamport_ts.is_some());
    if let Some(l) = m.lamport_ts {
        b.set_lamport_ts(l);
    }
}

pub(crate) fn read_synced_message(
    r: crate::community_message_capnp::synced_message::Reader<'_>,
) -> Result<SyncedMessage, ProtocolError> {
    Ok(SyncedMessage {
        sender_key: text_to_string(r.get_sender_key().map_err(|e| capnp_err(&e))?)?,
        body: text_to_string(r.get_body().map_err(|e| capnp_err(&e))?)?,
        timestamp: r.get_timestamp(),
        mek_generation: if r.get_has_mek_generation() {
            Some(r.get_mek_generation())
        } else {
            None
        },
        lamport_ts: if r.get_has_lamport_ts() {
            Some(r.get_lamport_ts())
        } else {
            None
        },
    })
}
