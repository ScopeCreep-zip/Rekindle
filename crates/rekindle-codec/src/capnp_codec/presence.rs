use super::{capnp_err, pack, read_game_status, unpack, write_game_status, CodecError};
use serde::{Deserialize, Serialize};

use crate::presence_capnp;

/// Game information for rich presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    /// Direct server address ("ip:port") for join-game functionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_address: Option<String>,
}

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

/// Decode packed bytes into `(status, Option<GameInfo>)`.
pub fn decode_update(data: &[u8]) -> Result<(u8, Option<GameInfo>), CodecError> {
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
pub fn decode_game_status(data: &[u8]) -> Result<GameInfo, CodecError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<presence_capnp::game_status::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    read_game_status(root)
}
