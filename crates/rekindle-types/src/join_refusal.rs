//! Why an inviter refused a joiner's `MemberJoined` (plan E3.2 join step 5,
//! ADR 0019's outcome rule: the Host names why an action did not run and
//! every frontend renders that one reason).
//!
//! The inviter's merge decides (`rekindle_governance::merge`'s invite gate)
//! and answers the reason in its `bootstrapRefused` reply. The reasons are
//! what the invite holder can already derive from the community's op DAG
//! its join walked (the `InviteCreated`'s `max_uses` and `expires_at`, and
//! any `InviteRevoked`), so naming them discloses nothing new. A revocation
//! and an expiry are one reason: whether a moderator acted against this
//! invite is not told apart from the clock (threat model: no signal of a
//! targeted action to a possibly hostile holder), and the remedy is the
//! same. Every other refusal (a banned author, an unapproved knock, a forged
//! proof) is `NotAdmitted`, never which of them.

use serde::{Deserialize, Serialize};

/// The inviter's typed refusal of a join.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JoinRefusal {
    /// Every use of the invite is taken by earlier joins.
    InviteUsedUp,
    /// The invite was revoked or has expired.
    InviteNoLongerValid,
    /// No live invite in the community matches the proof.
    InviteUnknown,
    /// The community does not admit this join.
    NotAdmitted,
}

impl JoinRefusal {
    /// The Host call's status code: one stable 4xx per reason.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::InviteUsedUp => 409,
            Self::InviteNoLongerValid => 410,
            Self::InviteUnknown => 404,
            Self::NotAdmitted => 403,
        }
    }

    /// The stable name of the reason.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::InviteUsedUp => "INVITE_USED_UP",
            Self::InviteNoLongerValid => "INVITE_NO_LONGER_VALID",
            Self::InviteUnknown => "INVITE_UNKNOWN",
            Self::NotAdmitted => "JOIN_NOT_ADMITTED",
        }
    }

    /// What the user is told.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::InviteUsedUp => "This invite has already been used.",
            Self::InviteNoLongerValid => "This invite is no longer valid.",
            Self::InviteUnknown => "This invite is not one the community knows.",
            Self::NotAdmitted => "The community did not admit this join.",
        }
    }

    /// What the user can do.
    #[must_use]
    pub const fn remediation(self) -> &'static str {
        match self {
            Self::NotAdmitted => "Ask a member of the community.",
            _ => "Ask for a new invite.",
        }
    }

    /// The reason's wire byte in `bootstrapRefused`.
    #[must_use]
    pub const fn to_wire(self) -> u8 {
        match self {
            Self::InviteUsedUp => 1,
            Self::InviteNoLongerValid => 2,
            Self::InviteUnknown => 3,
            Self::NotAdmitted => 4,
        }
    }

    /// The reason a wire byte names; an unknown byte is `NotAdmitted`.
    #[must_use]
    pub const fn from_wire(byte: u8) -> Self {
        match byte {
            1 => Self::InviteUsedUp,
            2 => Self::InviteNoLongerValid,
            3 => Self::InviteUnknown,
            _ => Self::NotAdmitted,
        }
    }
}

impl std::fmt::Display for JoinRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::JoinRefusal;

    #[test]
    fn every_reason_round_trips_its_wire_byte() {
        for r in [
            JoinRefusal::InviteUsedUp,
            JoinRefusal::InviteNoLongerValid,
            JoinRefusal::InviteUnknown,
            JoinRefusal::NotAdmitted,
        ] {
            assert_eq!(JoinRefusal::from_wire(r.to_wire()), r);
            assert!((400..500).contains(&r.code()));
        }
    }
}
