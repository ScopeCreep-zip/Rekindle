//! Message-plane leaf codecs: presence game info, thread
//! info, game servers, synced messages.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community::envelope::PresenceGameInfo;
use crate::error::CodecError;
use rekindle_types::game_server::GameServerInfo;
use rekindle_types::message::SyncedMessage;
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
) -> Result<PresenceGameInfo, CodecError> {
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
) -> Result<ThreadInfo, CodecError> {
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
) -> Result<GameServerInfo, CodecError> {
    Ok(GameServerInfo {
        id: text_to_string(r.get_id().map_err(|e| capnp_err(&e))?)?,
        game_id: text_to_string(r.get_game_id().map_err(|e| capnp_err(&e))?)?,
        label: text_to_string(r.get_label().map_err(|e| capnp_err(&e))?)?,
        address: text_to_string(r.get_address().map_err(|e| capnp_err(&e))?)?,
        added_by: text_to_string(r.get_added_by().map_err(|e| capnp_err(&e))?)?,
        created_at: r.get_created_at(),
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
) -> Result<SyncedMessage, CodecError> {
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
