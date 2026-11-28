//! The session-archive policy of a [`SessionRecord`] (plan E2.5h), built
//! from Sesame (§3: a record holds one active session and inactive ones,
//! newest first; a message for an inactive session is decrypted by it) and
//! Double Ratchet rev. 4 §8.4 (old state is deleted on a bound):
//!
//! - The archive holds at most [`MAX_INACTIVE_SESSIONS`]; archiving one
//!   more drops the oldest, whose keys are then erased (the record write
//!   that drops it marks the erasure, `rekindle_db::erase`), and clears the
//!   archived session's pending init (an archived session is never sent on).
//! - An inactive session that decrypts is written back where it is; it is
//!   never made active again. The active session changes only by our own
//!   initiation, a peer's initiation placed active, or the breaker.
//! - A peer's initiation goes active when we have none, when the peer has
//!   already acknowledged ours, or when its base key is greater (crossing
//!   initiations settle on the same session on both sides); otherwise it is
//!   archived, its first message delivered.
//! - The record counts the changes of direction between sending on the
//!   active session and receiving on an inactive one. Past
//!   [`MAX_DIRECTION_CHANGES`] the active session is archived: the two
//!   sides are oscillating between sessions, and the next send opens a new
//!   one, which the caller shows to the user.

use super::{Session, SessionRecord};

/// Inactive sessions kept per record.
pub const MAX_INACTIVE_SESSIONS: usize = 3;

/// Changes of direction tolerated per record before the active session is
/// archived.
pub const MAX_DIRECTION_CHANGES: u8 = 5;

/// How long a session the peer has not acknowledged is sent on, every
/// message carrying the PreKeyInit's fields (PQXDH rev. 3 §3.3: the
/// responder can build the session from any of them). Past it, the session
/// is not sent on and the next send opens a new one.
pub const PREKEY_WRAP_MAX_AGE_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

/// The record's last step, for counting changes of direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LastAct {
    /// Nothing since our own initiation.
    Unset,
    Send,
    ReceiveActive,
    ReceiveInactive,
}

impl LastAct {
    pub(super) fn to_byte(self) -> u8 {
        match self {
            Self::Unset => 0,
            Self::Send => 1,
            Self::ReceiveActive => 2,
            Self::ReceiveInactive => 3,
        }
    }

    pub(super) fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => Self::Unset,
            1 => Self::Send,
            2 => Self::ReceiveActive,
            3 => Self::ReceiveInactive,
            _ => return None,
        })
    }
}

/// Where a peer's initiation goes once its first message decrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Placement {
    Active,
    Archived,
}

impl SessionRecord {
    /// Archive `session`: the oldest inactive one leaves first when the
    /// archive is full, and the archived session's pending init is cleared.
    pub(super) fn archive(&mut self, mut session: Session) {
        while self.inactive.len() >= MAX_INACTIVE_SESSIONS {
            self.inactive.pop();
        }
        session.pending = None;
        self.inactive.insert(0, session);
    }

    /// Our own initiation: `session` is active, the counter starts again.
    pub(super) fn on_initiate(&mut self, session: Session) {
        if let Some(previous) = self.active.replace(session) {
            self.archive(previous);
        }
        self.direction_changes = 0;
        self.last_act = LastAct::Unset;
    }

    /// A message was sent on the active session.
    pub(super) fn on_send(&mut self) {
        if self.last_act == LastAct::ReceiveInactive {
            self.direction_changes = self.direction_changes.saturating_add(1);
        }
        self.last_act = LastAct::Send;
    }

    /// A message decrypted on the active session.
    pub(super) fn on_receive_active(&mut self) {
        self.last_act = LastAct::ReceiveActive;
    }

    /// A message decrypted on an inactive session; past
    /// [`MAX_DIRECTION_CHANGES`] the active session is archived.
    pub(super) fn on_receive_inactive(&mut self) {
        if self.last_act == LastAct::Send {
            self.direction_changes = self.direction_changes.saturating_add(1);
        }
        self.last_act = LastAct::ReceiveInactive;
        if self.direction_changes > MAX_DIRECTION_CHANGES {
            if let Some(active) = self.active.take() {
                self.archive(active);
            }
            self.direction_changes = 0;
        }
    }

    /// Where a peer's initiation with `incoming_base_key` goes. The record
    /// is for one identity (identity == address), so an initiation under a
    /// changed identity is another record's, which has no active session
    /// and takes it as active.
    pub(super) fn placement(&self, incoming_base_key: &[u8; 32]) -> Placement {
        match &self.active {
            None => Placement::Active,
            Some(active) if active.pending.is_none() => Placement::Active,
            Some(active) if incoming_base_key > active.state.base_key() => Placement::Active,
            Some(_) => Placement::Archived,
        }
    }

    /// Commit a peer's initiation whose first message decrypted.
    pub(super) fn commit_received(&mut self, session: Session, placement: Placement) {
        match placement {
            Placement::Active => {
                if let Some(previous) = self.active.replace(session) {
                    self.archive(previous);
                }
                self.direction_changes = 0;
                self.on_receive_active();
            }
            Placement::Archived => {
                self.archive(session);
                self.on_receive_inactive();
            }
        }
    }
}
