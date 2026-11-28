//! Message-plane leaf codecs: game servers, synced messages.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::error::CodecError;
use rekindle_types::game_server::GameServerInfo;

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
