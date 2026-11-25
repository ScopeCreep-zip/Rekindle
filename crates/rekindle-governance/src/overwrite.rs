//! The one canonical key of a channel permission overwrite's target.
//!
//! An overwrite (architecture §9.2 steps 4–6) targets a role or a member.
//! Its target travels as `(target_type, target_id)` text through IPC, the
//! governance op and the merged state, so the writer, the merge and every
//! reader (`compute_permissions`, `channel_read_roster`) must agree on one
//! spelling or an overwrite silently applies to no one. That spelling is
//! the API form of the id: a role's 32 lowercase hex characters
//! (`RoleId::to_hex`, `@everyone` included) and a member's 64
//! (`PseudonymKey::to_hex`). [`OverwriteTarget::parse`] accepts exactly
//! that form and nothing else, and [`OverwriteTarget::key`] produces it.

use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::key_format;

/// Whom a channel permission overwrite applies to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OverwriteTarget {
    /// Every holder of the role (`RoleId::EVERYONE` is every member).
    Role(RoleId),
    /// One member, by pseudonym.
    Member(PseudonymKey),
}

impl OverwriteTarget {
    /// The `target_type` text of a role target.
    pub const ROLE: &'static str = "role";
    /// The `target_type` text of a member target.
    pub const MEMBER: &'static str = "member";

    /// Parse an overwrite's `(target_type, target_id)`; `None` unless the
    /// type is `"role"` or `"member"` and the id is that target's
    /// canonical lowercase hex (32 characters for a role, 64 for a
    /// member). A non-canonical spelling is refused, never normalised,
    /// so the stored key is always the one readers look up.
    #[must_use]
    pub fn parse(target_type: &str, target_id: &str) -> Option<Self> {
        let target = match target_type {
            Self::ROLE => Self::Role(RoleId::from_hex(target_id)?),
            Self::MEMBER => {
                key_format::pseudonym_hex(target_id).ok()?;
                Self::Member(PseudonymKey::from_hex_lossy(target_id))
            }
            _ => return None,
        };
        (target.key() == target_id).then_some(target)
    }

    /// The overwrite map's target key (`GovernanceState::overwrites`).
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Role(role) => role.to_hex(),
            Self::Member(member) => member.to_hex(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_target_is_its_32_hex_and_a_member_its_64() {
        let everyone = OverwriteTarget::Role(RoleId::EVERYONE);
        assert_eq!(everyone.key(), "0".repeat(32));
        assert_eq!(
            OverwriteTarget::parse("role", &"0".repeat(32)),
            Some(everyone.clone())
        );
        let member = OverwriteTarget::Member(PseudonymKey([0xab; 32]));
        assert_eq!(member.key(), "ab".repeat(32));
        assert_eq!(
            OverwriteTarget::parse("member", &"ab".repeat(32)),
            Some(member.clone())
        );
    }

    #[test]
    fn a_malformed_or_non_canonical_target_is_refused() {
        for (ty, id) in [
            ("role", format!("{:?}", RoleId::EVERYONE)),
            ("role", "AB".repeat(16)),
            ("role", "ab".repeat(32)),
            ("role", String::new()),
            ("member", "ab".repeat(16)),
            ("member", "zz".repeat(32)),
            ("everyone", "0".repeat(32)),
        ] {
            assert_eq!(OverwriteTarget::parse(ty, &id), None, "{ty} {id}");
        }
    }
}
