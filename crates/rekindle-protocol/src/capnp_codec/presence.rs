use super::{capnp_err, pack, read_game_status, unpack, write_game_status, ProtocolError};
use crate::messaging::envelope::GameInfo;
use crate::presence_capnp;

/// Encode a presence update (status byte + optional game info).
pub fn encode_update(status: u8, game: Option<&GameInfo>) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<presence_capnp::presence_update::Builder<'_>>();
        root.set_status(status);
        root.set_timestamp(rekindle_utils::timestamp_ms());
        if let Some(g) = game {
            write_game_status(root.init_game_status(), g);
        }
    }
    pack(&builder)
}

/// Decode packed bytes into (status, Option<GameInfo>).
pub fn decode_update(data: &[u8]) -> Result<(u8, Option<GameInfo>), ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<presence_capnp::presence_update::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    let status = root.get_status();
    let game = if root.has_game_status() {
        let gs = root.get_game_status().map_err(|e| capnp_err(&e))?;
        Some(read_game_status(gs)?)
    } else {
        None
    };

    Ok((status, game))
}

/// Encode a standalone `GameStatus` (used for DHT subkey 4).
pub fn encode_game_status(info: &GameInfo) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let root = builder.init_root::<presence_capnp::game_status::Builder<'_>>();
        write_game_status(root, info);
    }
    pack(&builder)
}

/// Decode packed bytes into a `GameInfo`.
pub fn decode_game_status(data: &[u8]) -> Result<GameInfo, ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<presence_capnp::game_status::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    read_game_status(root)
}
