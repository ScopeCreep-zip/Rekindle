//! Capability enum and CapabilitySet lattice.
//!
//! `CapabilitySet` is a `BTreeSet<Capability>` with lattice operations
//! (union, intersection, subset). Delegation enforces monotonic
//! attenuation: `effective = delegator_scope.intersection(grant.scope)`.
//!
//! `Capability` derives `Ord` so `BTreeSet` iteration order is
//! deterministic, which gives deterministic wire encoding.

use std::collections::BTreeSet;

/// A single capability that a peer may exercise.
///
/// The `Ord` derive is required for `BTreeSet` (deterministic iteration
/// order → deterministic wire encoding). The discriminant ordering is
/// stable — reordering variants is a wire break.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum Capability {
    /// Send messages to channels.
    SendMessages,
    /// Read messages from channels.
    ReadMessages,
    /// Create, delete, update channels.
    ManageChannels,
    /// Kick, ban, timeout members.
    ModerateContent,
    /// Approve/reject join requests, manage member roles.
    ManageMembers,
    /// Re-delegate authority to another peer. `max_depth` limits
    /// the remaining chain length from this point.
    Delegate { max_depth: u8 },
    /// Application-defined capability with a validated name.
    /// Grammar: `[a-z0-9.-]{1,64}`.
    Custom(CustomCapability),
}

/// A validated custom capability name.
///
/// Grammar: `[a-z0-9.-]{1,64}`. Validated at construction; invalid
/// names cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CustomCapability(String);

impl CustomCapability {
    /// Construct from a string, validating the grammar.
    pub fn new(name: &str) -> Result<Self, crate::error::IdentityError> {
        if name.is_empty() || name.len() > 64 {
            return Err(crate::error::IdentityError::Encoding(
                "custom capability name must be 1-64 chars".into()
            ));
        }
        if !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-') {
            return Err(crate::error::IdentityError::Encoding(
                "custom capability name must match [a-z0-9.-]+".into()
            ));
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An ordered set of capabilities forming a lattice with union/intersection.
///
/// Delegation narrows scope via intersection — a delegate can never
/// exceed its delegator's capabilities.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilitySet {
    capabilities: BTreeSet<Capability>,
}

impl CapabilitySet {
    /// The empty set — no permissions.
    pub fn empty() -> Self {
        Self { capabilities: BTreeSet::new() }
    }

    /// Construct from an iterator of capabilities.
    pub fn from_iter(iter: impl IntoIterator<Item = Capability>) -> Self {
        Self { capabilities: iter.into_iter().collect() }
    }

    /// Set union: all capabilities from both sets.
    pub fn union(&self, other: &Self) -> Self {
        Self {
            capabilities: self.capabilities.union(&other.capabilities).cloned().collect(),
        }
    }

    /// Set intersection: only capabilities present in both sets.
    pub fn intersection(&self, other: &Self) -> Self {
        Self {
            capabilities: self.capabilities.intersection(&other.capabilities).cloned().collect(),
        }
    }

    /// True if every capability in `self` is also in `other`.
    pub fn is_subset(&self, other: &Self) -> bool {
        self.capabilities.is_subset(&other.capabilities)
    }

    /// True if every capability in `other` is also in `self`.
    pub fn is_superset(&self, other: &Self) -> bool {
        self.capabilities.is_superset(&other.capabilities)
    }

    /// True if the set has no capabilities.
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Number of capabilities.
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Whether the set contains the `Delegate` capability with sufficient depth.
    pub fn has_delegate(&self, required_depth: u8) -> bool {
        self.capabilities.iter().any(|c| matches!(c, Capability::Delegate { max_depth } if *max_depth >= required_depth))
    }

    /// Iterate capabilities in `Ord` order (deterministic for wire encoding).
    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.capabilities.iter()
    }

    /// Insert a capability.
    pub fn insert(&mut self, cap: Capability) {
        self.capabilities.insert(cap);
    }

    /// Check if a specific capability is present.
    pub fn contains(&self, cap: &Capability) -> bool {
        self.capabilities.contains(cap)
    }

    /// Encode capabilities for the wire signable form.
    ///
    /// Each capability encodes as a discriminant uint, except
    /// Delegate (which carries max_depth) and Custom (which carries
    /// the name string). The set encodes as a CBOR array in `Ord` order.
    pub fn encode_into(&self, buf: &mut Vec<u8>) {
        use crate::wire::encode;
        encode::array_head(buf, self.capabilities.len() as u64);
        for cap in &self.capabilities {
            match cap {
                Capability::SendMessages => encode::unsigned(buf, 0),
                Capability::ReadMessages => encode::unsigned(buf, 1),
                Capability::ManageChannels => encode::unsigned(buf, 2),
                Capability::ModerateContent => encode::unsigned(buf, 3),
                Capability::ManageMembers => encode::unsigned(buf, 4),
                Capability::Delegate { max_depth } => {
                    // 2-element array: [5, max_depth]
                    encode::array_head(buf, 2);
                    encode::unsigned(buf, 5);
                    encode::unsigned(buf, *max_depth as u64);
                }
                Capability::Custom(name) => {
                    // 2-element array: [15, name_string]
                    encode::array_head(buf, 2);
                    encode::unsigned(buf, 15);
                    encode::text(buf, name.as_str());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set() {
        let s = CapabilitySet::empty();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn intersection_monotone_attenuation() {
        let delegator = CapabilitySet::from_iter([
            Capability::SendMessages,
            Capability::ReadMessages,
            Capability::ManageChannels,
        ]);
        let grant = CapabilitySet::from_iter([
            Capability::ReadMessages,
            Capability::ManageChannels,
            Capability::ModerateContent,
        ]);
        let effective = delegator.intersection(&grant);
        assert_eq!(effective.len(), 2);
        assert!(effective.contains(&Capability::ReadMessages));
        assert!(effective.contains(&Capability::ManageChannels));
        assert!(!effective.contains(&Capability::SendMessages));
        assert!(!effective.contains(&Capability::ModerateContent));
    }

    #[test]
    fn intersection_with_empty_is_empty() {
        let full = CapabilitySet::from_iter([
            Capability::SendMessages,
            Capability::ReadMessages,
        ]);
        let empty = CapabilitySet::empty();
        assert!(full.intersection(&empty).is_empty());
    }

    #[test]
    fn subset_check() {
        let small = CapabilitySet::from_iter([Capability::ReadMessages]);
        let big = CapabilitySet::from_iter([
            Capability::SendMessages,
            Capability::ReadMessages,
        ]);
        assert!(small.is_subset(&big));
        assert!(!big.is_subset(&small));
    }

    #[test]
    fn has_delegate_with_sufficient_depth() {
        let set = CapabilitySet::from_iter([
            Capability::SendMessages,
            Capability::Delegate { max_depth: 3 },
        ]);
        assert!(set.has_delegate(1));
        assert!(set.has_delegate(3));
        assert!(!set.has_delegate(4));
    }

    #[test]
    fn custom_capability_grammar() {
        assert!(CustomCapability::new("my-cap.v1").is_ok());
        assert!(CustomCapability::new("abc123").is_ok());
        assert!(CustomCapability::new("a").is_ok());

        assert!(CustomCapability::new("").is_err());
        assert!(CustomCapability::new("UPPER").is_err());
        assert!(CustomCapability::new("has space").is_err());
        assert!(CustomCapability::new("has_underscore").is_err());
        assert!(CustomCapability::new(&"a".repeat(65)).is_err());
    }

    #[test]
    fn deterministic_iteration_order() {
        // Insert in random order, iterate in Ord order
        let a = CapabilitySet::from_iter([
            Capability::ManageChannels,
            Capability::SendMessages,
            Capability::ReadMessages,
        ]);
        let b = CapabilitySet::from_iter([
            Capability::ReadMessages,
            Capability::SendMessages,
            Capability::ManageChannels,
        ]);
        let a_vec: Vec<_> = a.iter().collect();
        let b_vec: Vec<_> = b.iter().collect();
        assert_eq!(a_vec, b_vec, "iteration order must be deterministic regardless of insertion order");
    }

    #[test]
    fn encode_deterministic() {
        let a = CapabilitySet::from_iter([
            Capability::ManageChannels,
            Capability::SendMessages,
        ]);
        let b = CapabilitySet::from_iter([
            Capability::SendMessages,
            Capability::ManageChannels,
        ]);
        let mut buf_a = Vec::new();
        let mut buf_b = Vec::new();
        a.encode_into(&mut buf_a);
        b.encode_into(&mut buf_b);
        assert_eq!(buf_a, buf_b, "wire encoding must be deterministic");
    }

    #[test]
    fn serde_roundtrip() {
        let set = CapabilitySet::from_iter([
            Capability::SendMessages,
            Capability::Delegate { max_depth: 2 },
            Capability::Custom(CustomCapability::new("my-app.read").unwrap()),
        ]);
        let json = serde_json::to_string(&set).unwrap();
        let restored: CapabilitySet = serde_json::from_str(&json).unwrap();
        assert_eq!(set, restored);
    }
}
