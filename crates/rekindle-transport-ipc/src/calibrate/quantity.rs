//! Physical quantities — the closed vocabulary of what a measurement measures.
//!
//! Each quantity fixes the unit semantics and the "higher is better/worse"
//! direction. The unit is a function of the quantity and MUST NOT appear
//! in the identifier — it is derived, not encoded.

/// The physical dimension of a calibration measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quantity {
    /// Bandwidth — bytes/second. Higher is better.
    Bw,
    /// Latency — seconds/operation. Lower is better.
    Lat,
    /// Rate — operations/second. Higher is better.
    Ops,
    /// Resource cost per operation — parameterized by `res` condition.
    /// Lower is better.
    Cost,
}

impl Quantity {
    /// The canonical lowercase token.
    pub fn token(self) -> &'static str {
        match self {
            Self::Bw => "bw",
            Self::Lat => "lat",
            Self::Ops => "ops",
            Self::Cost => "cost",
        }
    }

    /// Parse from token. Case-insensitive input, rejects unregistered.
    pub fn from_token(s: &str) -> Option<Self> {
        match s {
            "bw" => Some(Self::Bw),
            "lat" => Some(Self::Lat),
            "ops" => Some(Self::Ops),
            "cost" => Some(Self::Cost),
            _ => None,
        }
    }

    /// Whether higher values represent better performance.
    pub fn higher_is_better(self) -> bool {
        matches!(self, Self::Bw | Self::Ops)
    }

    /// The canonical unit family for the record's `unit` field.
    pub fn unit_family(self) -> &'static str {
        match self {
            Self::Bw => "bytes/sec",
            Self::Lat => "sec/op",
            Self::Ops => "ops/sec",
            Self::Cost => "resource/op",
        }
    }

    /// Condition keys that MUST be present for this quantity.
    pub fn required_conditions(self) -> &'static [&'static str] {
        match self {
            Self::Cost => &["res"],
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tokens_roundtrip() {
        for q in [Quantity::Bw, Quantity::Lat, Quantity::Ops, Quantity::Cost] {
            assert_eq!(Quantity::from_token(q.token()), Some(q));
        }
    }

    #[test]
    fn unregistered_returns_none() {
        assert_eq!(Quantity::from_token("xyz"), None);
        assert_eq!(Quantity::from_token(""), None);
    }

    #[test]
    fn higher_is_better_semantics() {
        assert!(Quantity::Bw.higher_is_better());
        assert!(Quantity::Ops.higher_is_better());
        assert!(!Quantity::Lat.higher_is_better());
        assert!(!Quantity::Cost.higher_is_better());
    }

    #[test]
    fn cost_requires_res() {
        assert_eq!(Quantity::Cost.required_conditions(), &["res"]);
        assert!(Quantity::Bw.required_conditions().is_empty());
    }
}
