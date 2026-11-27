//! The Xfire surface's bodies (plan E7.2): our bio and game totals, shared
//! with friends.

use crate::capnp_codec::{capnp_err, CodecError};
use crate::direct_capnp::{game_invite, game_totals, profile_bio};
use rekindle_types::game_server::ServerAddress;

/// 0x0170 GameInvite: join us on `server` in game `game_id`. The decoder
/// parses `server` with [`ServerAddress::parse`], so an address that could
/// carry a path, query or launch argument never leaves the codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInvite {
    pub game_id: u32,
    pub server: ServerAddress,
}

impl GameInvite {
    pub(super) fn write(&self, mut b: game_invite::Builder<'_>) {
        b.set_game_id(self.game_id);
        b.set_server(self.server.as_str());
    }

    pub(super) fn read(r: game_invite::Reader<'_>) -> Result<Self, CodecError> {
        let game_id = r.get_game_id();
        if game_id == 0 {
            return Err(CodecError::Deserialization(
                "GameInvite names game 0".into(),
            ));
        }
        let text = r
            .get_server()
            .map_err(|e| capnp_err(&e))?
            .to_str()
            .map_err(|e| CodecError::Deserialization(format!("GameInvite: {e}")))?;
        let server = ServerAddress::parse(text)
            .map_err(|e| CodecError::Deserialization(format!("GameInvite server: {e}")))?;
        Ok(Self { game_id, server })
    }
}

/// 0x0171 ProfileBio: our bio, `None` clearing it. The decoder holds it to
/// `rekindle_types::presence::limits::bio` (190 characters, no control
/// characters); blank reads as `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileBio {
    pub bio: Option<String>,
}

impl ProfileBio {
    pub(super) fn write(&self, mut b: profile_bio::Builder<'_>) {
        b.set_bio(self.bio.as_deref().unwrap_or(""));
    }

    pub(super) fn read(r: profile_bio::Reader<'_>) -> Result<Self, CodecError> {
        let text = r
            .get_bio()
            .map_err(|e| capnp_err(&e))?
            .to_str()
            .map_err(|e| CodecError::Deserialization(format!("ProfileBio: {e}")))?;
        let bio = rekindle_types::presence::limits::bio(text)
            .map_err(|e| CodecError::Deserialization(format!("ProfileBio: {e}")))?;
        Ok(Self {
            bio: bio.map(str::to_string),
        })
    }
}

/// The most games one `GameTotals` names.
pub const MAX_GAME_TOTALS: usize = 512;

pub use rekindle_types::dto::presence::GameTotal;

/// 0x0172 GameTotals: every game we played, replacing what the friend held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameTotals {
    pub totals: Vec<GameTotal>,
}

impl GameTotals {
    pub(super) fn write(&self, b: game_totals::Builder<'_>) -> Result<(), CodecError> {
        let mut list = b.init_totals(crate::capnp_codec::wire_len(self.totals.len())?);
        for (i, t) in (0u32..).zip(&self.totals) {
            let mut e = list.reborrow().get(i);
            e.set_game_id(t.game_id);
            e.set_total_secs(t.total_secs);
            e.set_last_played(t.last_played);
        }
        Ok(())
    }

    pub(super) fn read(r: game_totals::Reader<'_>) -> Result<Self, CodecError> {
        let list = r.get_totals().map_err(|e| capnp_err(&e))?;
        if list.len() as usize > MAX_GAME_TOTALS {
            return Err(CodecError::Deserialization(format!(
                "GameTotals names {} games (cap {MAX_GAME_TOTALS})",
                list.len()
            )));
        }
        let mut totals = Vec::with_capacity(list.len() as usize);
        for e in list {
            if e.get_game_id() == 0 {
                return Err(CodecError::Deserialization(
                    "GameTotals names game 0".into(),
                ));
            }
            if totals
                .iter()
                .any(|t: &GameTotal| t.game_id == e.get_game_id())
            {
                return Err(CodecError::Deserialization(format!(
                    "GameTotals names game {} twice",
                    e.get_game_id()
                )));
            }
            totals.push(GameTotal {
                game_id: e.get_game_id(),
                total_secs: e.get_total_secs(),
                last_played: e.get_last_played(),
            });
        }
        Ok(Self { totals })
    }
}

/// An invite sample for the every-body round trip.
#[cfg(test)]
pub(crate) fn invite_sample() -> GameInvite {
    GameInvite {
        game_id: 4181,
        server: ServerAddress::parse("203.0.113.7:27015").unwrap(),
    }
}

/// A bio sample for the every-body round trip.
#[cfg(test)]
pub(crate) fn bio_sample() -> ProfileBio {
    ProfileBio {
        bio: Some("Quake 3 since '99".into()),
    }
}

/// A two-game sample for the every-body round trip.
#[cfg(test)]
pub(crate) fn sample() -> GameTotals {
    GameTotals {
        totals: vec![
            GameTotal {
                game_id: 4181,
                total_secs: 5_400,
                last_played: 1_700_000_000_000,
            },
            GameTotal {
                game_id: 4102,
                total_secs: 0,
                last_played: 1_600_000_000_000,
            },
        ],
    }
}

#[cfg(test)]
#[path = "game_tests.rs"]
mod tests;
