//! The Xfire surface's bodies (plan E7.2): game totals shared with friends.

use crate::capnp_codec::{capnp_err, CodecError};
use crate::direct_capnp::game_totals;

/// The most games one `GameTotals` names.
pub const MAX_GAME_TOTALS: usize = 512;

pub use rekindle_types::dto::presence::GameTotal;

/// 0x0172 GameTotals: every game we played, replacing what the friend held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameTotals {
    pub totals: Vec<GameTotal>,
}

impl GameTotals {
    pub(super) fn write(&self, b: game_totals::Builder<'_>) {
        let mut list = b.init_totals(u32::try_from(self.totals.len()).unwrap_or(u32::MAX));
        for (i, t) in (0u32..).zip(&self.totals) {
            let mut e = list.reborrow().get(i);
            e.set_game_id(t.game_id);
            e.set_total_secs(t.total_secs);
            e.set_last_played(t.last_played);
        }
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
mod tests {
    use super::*;
    use crate::capnp_codec::direct::{DirectBody, DmTypeId};

    /// Plan E7.2: a `GameTotals` naming game 0, a game twice, or more games
    /// than the cap is refused on decode.
    #[test]
    fn bad_game_totals_are_refused() {
        let t = |game_id| GameTotal {
            game_id,
            total_secs: 1,
            last_played: 1,
        };
        for totals in [
            vec![t(0)],
            vec![t(7), t(7)],
            (1..=u32::try_from(MAX_GAME_TOTALS).unwrap() + 1)
                .map(t)
                .collect(),
        ] {
            let bytes = DirectBody::GameTotals(GameTotals { totals }).encode();
            assert!(DirectBody::decode(DmTypeId::GameTotals, &bytes).is_err());
        }
    }
}
