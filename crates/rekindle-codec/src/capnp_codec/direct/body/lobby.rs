//! Game-lobby bodies (plan E7.2 G6, SK2-G §5): a lobby is state inside one
//! group DM, so each kind is sealed only inside a `GroupContent` entry's
//! inner (its group is that entry's) and never travels as a frame of its
//! own. Every member folds them into `group_lobbies` (`rekindle_db::repo::
//! group_lobby`); a kind from a writer off the group's roster is dropped
//! (`rekindle_dm::admission::admit_lobby`).

use rekindle_types::game_server::ServerAddress;

use crate::capnp_codec::direct::fixed;
use crate::capnp_codec::{capnp_err, CodecError};
use crate::direct_capnp::{lobby_launch, lobby_open, lobby_ref};

/// A lobby's id: 16 random bytes its host mints.
pub type LobbyId = [u8; 16];

/// 0x0127 LobbyOpen: the writer opens lobby `lobby_id` for game `game_id`,
/// as its host. Game 0 is malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyOpen {
    pub lobby_id: LobbyId,
    pub game_id: u32,
}

impl LobbyOpen {
    pub(super) fn write(&self, mut b: lobby_open::Builder<'_>) {
        b.set_lobby_id(&self.lobby_id);
        b.set_game_id(self.game_id);
    }

    pub(super) fn read(r: lobby_open::Reader<'_>) -> Result<Self, CodecError> {
        let game_id = r.get_game_id();
        if game_id == 0 {
            return Err(CodecError::Deserialization("LobbyOpen names game 0".into()));
        }
        Ok(Self {
            lobby_id: fixed(r.get_lobby_id(), "lobby id")?,
            game_id,
        })
    }
}

/// The body of 0x0128 LobbyJoin, 0x0129 LobbyLeave and 0x012B LobbyClose:
/// the lobby the writer joins, leaves or (its host) closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyRef {
    pub lobby_id: LobbyId,
}

impl LobbyRef {
    pub(super) fn write(&self, mut b: lobby_ref::Builder<'_>) {
        b.set_lobby_id(&self.lobby_id);
    }

    pub(super) fn read(r: lobby_ref::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            lobby_id: fixed(r.get_lobby_id(), "lobby id")?,
        })
    }
}

/// 0x012A LobbyLaunch: the host launches its lobby on `server`. The decoder
/// parses `server` with [`ServerAddress::parse`] (as `GameInvite`'s), so
/// an address that could carry a path, query or launch argument drops the
/// frame and never leaves the codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyLaunch {
    pub lobby_id: LobbyId,
    pub server: ServerAddress,
}

impl LobbyLaunch {
    pub(super) fn write(&self, mut b: lobby_launch::Builder<'_>) {
        b.set_lobby_id(&self.lobby_id);
        b.set_server(self.server.as_str());
    }

    pub(super) fn read(r: lobby_launch::Reader<'_>) -> Result<Self, CodecError> {
        let text = r
            .get_server()
            .map_err(|e| capnp_err(&e))?
            .to_str()
            .map_err(|e| CodecError::Deserialization(format!("LobbyLaunch: {e}")))?;
        let server = ServerAddress::parse(text)
            .map_err(|e| CodecError::Deserialization(format!("LobbyLaunch server: {e}")))?;
        Ok(Self {
            lobby_id: fixed(r.get_lobby_id(), "lobby id")?,
            server,
        })
    }
}
