//! Message-plane leaf codecs: presence game info, game servers,
//! synced messages.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community::envelope::PresenceGameInfo;
use crate::error::CodecError;
use rekindle_types::game_server::GameServerInfo;

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
