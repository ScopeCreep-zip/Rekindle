//! Conformance profiles — the opt-in complexity lattice.
//!
//! Each profile is a superset of the one below. Higher profiles add
//! checks and outputs without changing what lower profiles accept.

/// Conformance level. Each level is a strict superset of the one below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Profile {
    /// Parse and canonicalize quantity + operation. Reject unregistered tokens.
    L0Core = 0,
    /// L0 + condition registry enforcement: typing, domain, required conditions.
    L1Conditions = 1,
    /// L1 + substrate auto-detection and mandatory population in emitted records.
    L2Substrate = 2,
    /// L2 + dependency-gated execution with skip-and-diagnosis.
    L3Accumulation = 3,
    /// L3 + validation predicate algebra with physics bounds.
    L4Validation = 4,
    /// L4 + full measurement record with provenance and columnar export.
    L5Provenance = 5,
}

impl Profile {
    /// Human-readable name for diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Self::L0Core => "L0 Core",
            Self::L1Conditions => "L1 Conditions",
            Self::L2Substrate => "L2 Substrate",
            Self::L3Accumulation => "L3 Accumulation",
            Self::L4Validation => "L4 Validation",
            Self::L5Provenance => "L5 Provenance",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_monotone() {
        assert!(Profile::L0Core < Profile::L1Conditions);
        assert!(Profile::L1Conditions < Profile::L2Substrate);
        assert!(Profile::L2Substrate < Profile::L3Accumulation);
        assert!(Profile::L3Accumulation < Profile::L4Validation);
        assert!(Profile::L4Validation < Profile::L5Provenance);
    }

    #[test]
    fn names_are_distinct() {
        let names: Vec<&str> = [
            Profile::L0Core, Profile::L1Conditions, Profile::L2Substrate,
            Profile::L3Accumulation, Profile::L4Validation, Profile::L5Provenance,
        ].iter().map(|p| p.name()).collect();
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
