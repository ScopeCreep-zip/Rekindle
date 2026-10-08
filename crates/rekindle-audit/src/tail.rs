//! The tail-anchor check: does the stored chain still end where the vault
//! says it ended?
//!
//! Every append writes its `(cursor, mac)` into the vault as well as the
//! database. Dropping the last rows leaves a chain that still verifies, just
//! shorter; only the vault's copy of the tail shows it happened. The vault is
//! a separate, encrypted file, so whoever can edit the database cannot move
//! the anchor with it.

use crate::MAC_LEN;

/// A chain end: the last cursor and its MAC.
pub type Tail = (u64, [u8; MAC_LEN]);

/// What comparing the stored tail with the vault anchor found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailCheck {
    /// No anchor yet (a new identity): the stored tail is all there is.
    Unanchored,
    /// The anchor and the stored tail agree.
    Clean,
    /// The anchor is behind: an append reached the database but its vault
    /// write was lost (logout or a crash between the two). Not tampering;
    /// a full verify still re-MACs every entry in the gap.
    CatchUp,
    /// The anchor is ahead, or names the same cursor with another MAC: rows
    /// were removed or the last one was rewritten. The anchor is the last
    /// entry known good.
    Tampered { anchor: Tail },
}

impl TailCheck {
    /// Compare the vault `anchor` with the `stored` tail.
    #[must_use]
    pub fn of(anchor: Option<Tail>, stored: Tail) -> Self {
        match anchor {
            None => Self::Unanchored,
            Some(a) if a == stored => Self::Clean,
            Some(a) if a.0 < stored.0 => Self::CatchUp,
            Some(anchor) => Self::Tampered { anchor },
        }
    }

    /// The tail the chain continues from: the anchor when the stored tail
    /// cannot be trusted, the stored tail otherwise.
    #[must_use]
    pub fn resume_from(self, stored: Tail) -> Tail {
        match self {
            Self::Tampered { anchor } => anchor,
            Self::Unanchored | Self::Clean | Self::CatchUp => stored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anchor_behind_the_store_is_a_catch_up() {
        assert_eq!(
            TailCheck::of(Some((3, [0xAA; MAC_LEN])), (5, [0xBB; MAC_LEN])),
            TailCheck::CatchUp
        );
    }

    #[test]
    fn an_anchor_ahead_of_the_store_is_tampering() {
        let anchor = (5, [0xAA; MAC_LEN]);
        let check = TailCheck::of(Some(anchor), (3, [0xBB; MAC_LEN]));
        assert_eq!(check, TailCheck::Tampered { anchor });
        assert_eq!(check.resume_from((3, [0xBB; MAC_LEN])), anchor);
    }

    #[test]
    fn the_same_cursor_with_another_mac_is_tampering() {
        let anchor = (5, [0xAA; MAC_LEN]);
        assert_eq!(
            TailCheck::of(Some(anchor), (5, [0xCC; MAC_LEN])),
            TailCheck::Tampered { anchor }
        );
    }

    #[test]
    fn an_equal_tail_is_clean_and_no_anchor_is_unanchored() {
        let tail = (5, [0xAA; MAC_LEN]);
        assert_eq!(TailCheck::of(Some(tail), tail), TailCheck::Clean);
        assert_eq!(TailCheck::of(None, tail), TailCheck::Unanchored);
    }
}
