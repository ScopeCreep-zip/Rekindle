//! OSC identifier parser, canonical form, and equivalence.
//!
//! The identifier is the primary key of a calibration measurement.
//! It encodes four dimensions: quantity, operation, conditions, substrate.
//! Two identifiers denote the same measurement if and only if their
//! canonical forms are byte-identical.
//!
//! The scheme is `osc:`. No aliases. No legacy.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use super::condition;
use super::operation::OperationPath;
use super::profile::Profile;
use super::quantity::Quantity;
use super::substrate::Substrate;

/// A parsed, validated, canonicalizable OSC identifier.
///
/// Fields are private. Construction is through `parse()` only.
/// Access is through typed accessors. This prevents construction
/// of identifiers that bypass registry validation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OscId {
    quantity: Quantity,
    operation: OperationPath,
    conditions: BTreeMap<String, String>,
    substrate: Option<Substrate>,
}

/// Errors from parsing an OSC identifier.
#[derive(Debug, Clone)]
pub enum ParseError {
    Empty,
    MissingScheme,
    InvalidScheme(String),
    MissingQuantity,
    UnregisteredQuantity(String),
    MissingOperation,
    InvalidOperation(String),
    UnregisteredOperation(String),
    DuplicateConditionKey(String),
    InvalidCondition(String),
    ConditionNotApplicable { key: String, domain: String },
    MissingRequiredCondition { key: String, context: String },
    InvalidSubstrate(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty identifier"),
            Self::MissingScheme => write!(f, "missing 'osc:' scheme"),
            Self::InvalidScheme(s) => write!(f, "invalid scheme '{s}', expected 'osc'"),
            Self::MissingQuantity => write!(f, "missing quantity after 'osc:'"),
            Self::UnregisteredQuantity(q) => write!(f, "unregistered quantity '{q}'"),
            Self::MissingOperation => write!(f, "missing operation after '/'"),
            Self::InvalidOperation(s) => write!(f, "invalid operation: {s}"),
            Self::UnregisteredOperation(op) => write!(f, "unregistered operation '{op}'"),
            Self::DuplicateConditionKey(k) => write!(f, "duplicate condition key '{k}'"),
            Self::InvalidCondition(s) => write!(f, "invalid condition: {s}"),
            Self::ConditionNotApplicable { key, domain } => {
                write!(f, "condition '{key}' not applicable to domain '{domain}'")
            }
            Self::MissingRequiredCondition { key, context } => {
                write!(f, "missing required condition '{key}' for {context}")
            }
            Self::InvalidSubstrate(s) => write!(f, "invalid substrate '{s}'"),
        }
    }
}

impl std::error::Error for ParseError {}

// ── Construction ────────────────────────────────────────────────────

impl OscId {
    /// Parse an OSC identifier at the given conformance profile.
    pub fn parse(s: &str, profile: Profile) -> Result<Self, ParseError> {
        if s.is_empty() {
            return Err(ParseError::Empty);
        }

        // Scheme: must be "osc:"
        let rest = match s.strip_prefix("osc:") {
            Some(r) => r,
            None => {
                return if s.contains(':') {
                    let scheme = &s[..s.find(':').unwrap()];
                    Err(ParseError::InvalidScheme(scheme.to_owned()))
                } else {
                    Err(ParseError::MissingScheme)
                };
            }
        };

        // Split off substrate (@... at the end)
        let (rest, substrate_str) = match rest.rfind('@') {
            Some(at) => (&rest[..at], Some(&rest[at + 1..])),
            None => (rest, None),
        };

        // Split off conditions (?... before @)
        let (rest, conditions_str) = match rest.find('?') {
            Some(q) => (&rest[..q], Some(&rest[q + 1..])),
            None => (rest, None),
        };

        // Split quantity / operation
        let slash = rest.find('/').ok_or(ParseError::MissingOperation)?;
        let quantity_str = &rest[..slash];
        let operation_str = &rest[slash + 1..];

        if quantity_str.is_empty() {
            return Err(ParseError::MissingQuantity);
        }
        if operation_str.is_empty() {
            return Err(ParseError::MissingOperation);
        }

        // Parse quantity
        let quantity = Quantity::from_token(&quantity_str.to_ascii_lowercase())
            .ok_or_else(|| ParseError::UnregisteredQuantity(quantity_str.to_owned()))?;

        // Parse operation
        let operation = OperationPath::parse(operation_str)
            .ok_or_else(|| ParseError::InvalidOperation(operation_str.to_owned()))?;

        // L0: check operation is registered
        if profile >= Profile::L0Core && !operation.all_registered() {
            let unreg: Vec<_> = operation.operands().iter()
                .filter(|op| !super::operation::SEEDED_OPERATIONS.contains(&op.as_str()))
                .cloned()
                .collect();
            return Err(ParseError::UnregisteredOperation(unreg.join(", ")));
        }

        // Parse conditions
        let conditions = parse_conditions(conditions_str)?;

        // L1: validate conditions
        if profile >= Profile::L1Conditions {
            for domain in operation.domains() {
                for (key, value) in &conditions {
                    condition::validate(key, value, domain)
                        .map_err(|e| ParseError::InvalidCondition(e))?;
                }
            }
            for req in quantity.required_conditions() {
                if !conditions.contains_key(*req) {
                    return Err(ParseError::MissingRequiredCondition {
                        key: req.to_string(),
                        context: format!("quantity '{}'", quantity.token()),
                    });
                }
            }
        }

        // Parse substrate
        let substrate = match substrate_str {
            Some(s) => Some(Substrate::parse(s)
                .ok_or_else(|| ParseError::InvalidSubstrate(s.to_owned()))?),
            None => None,
        };

        Ok(Self { quantity, operation, conditions, substrate })
    }

    // ── Canonical form ──────────────────────────────────────────────

    /// Emit canonical form. Conditions sorted by key. Operands sorted.
    /// Scheme is always `osc:`.
    pub fn canonical(&self) -> String {
        let mut out = String::with_capacity(64);
        out.push_str("osc:");
        out.push_str(self.quantity.token());
        out.push('/');
        out.push_str(&self.operation.canonical());

        if !self.conditions.is_empty() {
            out.push('?');
            let mut first = true;
            for (k, v) in &self.conditions {
                if !first { out.push('&'); }
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                first = false;
            }
        }

        if let Some(ref sub) = self.substrate {
            out.push('@');
            out.push_str(&sub.canonical());
        }

        out
    }

    /// Canonical form with substrate auto-populated if absent.
    pub fn canonical_for_emission(&self) -> String {
        if self.substrate.is_some() {
            return self.canonical();
        }
        let mut id = self.clone();
        id.substrate = Some(Substrate::auto_detect());
        id.canonical()
    }

    // ── Monotonicity projections ────────────────────────────────────

    /// Strip substrate. Result is always valid.
    pub fn strip_substrate(&self) -> Self {
        Self {
            quantity: self.quantity,
            operation: self.operation.clone(),
            conditions: self.conditions.clone(),
            substrate: None,
        }
    }

    /// Strip all conditions. Result is always valid.
    pub fn strip_conditions(&self) -> Self {
        Self {
            quantity: self.quantity,
            operation: self.operation.clone(),
            conditions: BTreeMap::new(),
            substrate: self.substrate.clone(),
        }
    }

    // ── Specificity ─────────────────────────────────────────────────

    /// Whether self is more specific than other (same quantity+operation,
    /// other's conditions are a subset, other's substrate is absent or equal).
    pub fn is_more_specific_than(&self, other: &Self) -> bool {
        if self.quantity != other.quantity || self.operation != other.operation {
            return false;
        }
        for (k, v) in &other.conditions {
            if self.conditions.get(k) != Some(v) {
                return false;
            }
        }
        match (&other.substrate, &self.substrate) {
            (Some(os), Some(ss)) => os == ss,
            (Some(_), None) => false,
            (None, _) => true,
        }
    }

    // ── Accessors ───────────────────────────────────────────────────

    pub fn quantity(&self) -> Quantity { self.quantity }
    pub fn operation(&self) -> &OperationPath { &self.operation }
    pub fn conditions(&self) -> &BTreeMap<String, String> { &self.conditions }
    pub fn substrate(&self) -> Option<&Substrate> { self.substrate.as_ref() }
    pub fn op_domain(&self) -> &str { self.operation.domain() }
}

impl fmt::Display for OscId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.canonical())
    }
}

impl FromStr for OscId {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s, Profile::L1Conditions)
    }
}

// ── Internal ────────────────────────────────────────────────────────

fn parse_conditions(raw: Option<&str>) -> Result<BTreeMap<String, String>, ParseError> {
    let mut map = BTreeMap::new();
    let raw = match raw {
        Some(r) if !r.is_empty() => r,
        _ => return Ok(map),
    };
    for pair in raw.split('&') {
        let eq = pair.find('=').ok_or_else(|| {
            ParseError::InvalidCondition(format!("missing '=' in '{pair}'"))
        })?;
        let key = pair[..eq].to_ascii_lowercase();
        let raw_value = pair[eq + 1..].to_ascii_lowercase();

        if key.is_empty() {
            return Err(ParseError::InvalidCondition("empty condition key".to_owned()));
        }
        if map.contains_key(&key) {
            return Err(ParseError::DuplicateConditionKey(key));
        }

        let value = if raw_value.contains('+') {
            condition::canonicalize_compound(&raw_value)
        } else {
            raw_value
        };

        map.insert(key, value);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Parser correctness 1-13
    #[test] fn parse_minimal() {
        let id = OscId::parse("osc:bw/mem.copy", Profile::L0Core).unwrap();
        assert_eq!(id.quantity(), Quantity::Bw);
        assert_eq!(id.operation().canonical(), "mem.copy");
    }
    #[test] fn parse_missing_scheme() {
        assert!(matches!(OscId::parse("bw/mem.copy", Profile::L0Core), Err(ParseError::MissingScheme)));
    }
    #[test] fn parse_invalid_scheme() {
        assert!(matches!(OscId::parse("cal:bw/mem.copy", Profile::L0Core), Err(ParseError::InvalidScheme(_))));
    }
    #[test] fn parse_unregistered_quantity() {
        assert!(matches!(OscId::parse("osc:xyz/mem.copy", Profile::L0Core), Err(ParseError::UnregisteredQuantity(_))));
    }
    #[test] fn parse_with_conditions() {
        let id = OscId::parse("osc:bw/mem.copy?size=16k&cache=l1", Profile::L1Conditions).unwrap();
        assert_eq!(id.conditions().len(), 2);
    }
    #[test] fn parse_duplicate_condition() {
        assert!(matches!(OscId::parse("osc:bw/mem.copy?cache=l1&cache=l2", Profile::L0Core), Err(ParseError::DuplicateConditionKey(_))));
    }
    #[test] fn parse_with_substrate() {
        let id = OscId::parse("osc:bw/mem.copy@x86_64.linux.uring", Profile::L0Core).unwrap();
        assert!(id.substrate().is_some());
    }
    #[test] fn parse_cost_missing_res() {
        assert!(matches!(OscId::parse("osc:cost/crypto.seal?size=1m", Profile::L1Conditions), Err(ParseError::MissingRequiredCondition { .. })));
    }
    #[test] fn parse_cost_with_res() {
        OscId::parse("osc:cost/crypto.seal?res=cycles&size=1m", Profile::L1Conditions).unwrap();
    }
    #[test] fn parse_empty() {
        assert!(matches!(OscId::parse("", Profile::L0Core), Err(ParseError::Empty)));
    }
    #[test] fn parse_scheme_only() {
        assert!(matches!(OscId::parse("osc:", Profile::L0Core), Err(ParseError::MissingOperation)));
    }
    #[test] fn parse_no_operation() {
        assert!(matches!(OscId::parse("osc:bw/", Profile::L0Core), Err(ParseError::MissingOperation)));
    }
    #[test] fn parse_no_slash() {
        assert!(matches!(OscId::parse("osc:bw", Profile::L0Core), Err(ParseError::MissingOperation)));
    }

    // Canonical form 14-19
    #[test] fn canonical_sorts_conditions() {
        let id = OscId::parse("osc:bw/mem.copy?size=16k&cache=l1", Profile::L0Core).unwrap();
        assert_eq!(id.canonical(), "osc:bw/mem.copy?cache=l1&size=16k");
    }
    #[test] fn canonical_sorts_composed_ops() {
        let id = OscId::parse("osc:lat/crypto.seal+crypto.mac", Profile::L0Core).unwrap();
        assert_eq!(id.canonical(), "osc:lat/crypto.mac+crypto.seal");
    }
    #[test] fn canonical_deduplicates_composed() {
        let id = OscId::parse("osc:lat/crypto.seal+crypto.seal", Profile::L0Core).unwrap();
        assert_eq!(id.canonical(), "osc:lat/crypto.seal");
    }
    #[test] fn canonical_lowercases() {
        // Input is already lowercase by parse, but verify
        let id = OscId::parse("osc:bw/mem.copy", Profile::L0Core).unwrap();
        assert!(id.canonical().chars().all(|c| !c.is_ascii_uppercase() || c == '@'));
    }
    #[test] fn canonical_sorts_compound_values() {
        let id = OscId::parse("osc:lat/sync.rwlock?contention=4t+1w", Profile::L0Core).unwrap();
        assert!(id.canonical().contains("contention=1w+4t"));
    }
    #[test] fn canonical_scheme_is_osc() {
        let id = OscId::parse("osc:bw/mem.copy", Profile::L0Core).unwrap();
        assert!(id.canonical().starts_with("osc:"));
    }

    // Round-trip 20-22
    #[test] fn roundtrip_canonical() {
        let input = "osc:bw/crypto.seal?alloc=none&size=64k&variant=aegis128l@x86_64.linux.uring";
        let id = OscId::parse(input, Profile::L0Core).unwrap();
        let c = id.canonical();
        let id2 = OscId::parse(&c, Profile::L0Core).unwrap();
        assert_eq!(id, id2);
        assert_eq!(c, id2.canonical());
    }
    #[test] fn display_is_canonical() {
        let id = OscId::parse("osc:bw/mem.copy?size=16k&cache=l1", Profile::L0Core).unwrap();
        assert_eq!(format!("{id}"), id.canonical());
    }
    #[test] fn fromstr_parses_l1() {
        let id: OscId = "osc:bw/mem.copy?size=16k&cache=l1".parse().unwrap();
        assert_eq!(id.quantity(), Quantity::Bw);
    }

    // Monotonicity 23-25
    #[test] fn strip_substrate_valid() {
        let id = OscId::parse("osc:bw/mem.copy?size=64m@x86_64.linux.uring", Profile::L0Core).unwrap();
        let stripped = id.strip_substrate();
        assert!(stripped.substrate().is_none());
        assert_eq!(stripped.canonical(), "osc:bw/mem.copy?size=64m");
    }
    #[test] fn strip_conditions_valid() {
        let id = OscId::parse("osc:bw/mem.copy?cache=l1&size=16k@x86_64.linux.uring", Profile::L0Core).unwrap();
        let stripped = id.strip_conditions();
        assert!(stripped.conditions().is_empty());
        assert_eq!(stripped.canonical(), "osc:bw/mem.copy@x86_64.linux.uring");
    }
    #[test] fn strip_both_gives_minimal_core() {
        let id = OscId::parse("osc:bw/mem.copy?cache=l1&size=16k@x86_64.linux.uring", Profile::L0Core).unwrap();
        let minimal = id.strip_substrate().strip_conditions();
        assert_eq!(minimal.canonical(), "osc:bw/mem.copy");
    }

    // Specificity 26-29
    #[test] fn more_conditions_more_specific() {
        let general = OscId::parse("osc:bw/mem.copy", Profile::L0Core).unwrap();
        let specific = OscId::parse("osc:bw/mem.copy?size=64k", Profile::L0Core).unwrap();
        assert!(specific.is_more_specific_than(&general));
        assert!(!general.is_more_specific_than(&specific));
    }
    #[test] fn substrate_more_specific() {
        let no_sub = OscId::parse("osc:bw/mem.copy?size=64k", Profile::L0Core).unwrap();
        let with_sub = OscId::parse("osc:bw/mem.copy?size=64k@x86_64.linux.uring", Profile::L0Core).unwrap();
        assert!(with_sub.is_more_specific_than(&no_sub));
    }
    #[test] fn reflexive_specificity() {
        let id = OscId::parse("osc:bw/mem.copy?size=64k", Profile::L0Core).unwrap();
        assert!(id.is_more_specific_than(&id));
    }
    #[test] fn different_ops_incomparable() {
        let a = OscId::parse("osc:bw/mem.copy", Profile::L0Core).unwrap();
        let b = OscId::parse("osc:bw/mem.read", Profile::L0Core).unwrap();
        assert!(!a.is_more_specific_than(&b));
        assert!(!b.is_more_specific_than(&a));
    }
}
