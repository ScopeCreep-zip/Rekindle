//! Typed condition key=value pairs for measurement parameterization.
//!
//! Each condition key has a fixed type (enum, magnitude, cardinality, token,
//! compound) and a set of operation domains it applies to. A condition key
//! applied to an operation outside its domain is invalid.

/// Condition value type — determines parsing and canonicalization rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionType {
    /// A closed set of value tokens.
    Enum,
    /// IEC byte magnitude: 64, 1k, 64k, 1m, 16m.
    Magnitude,
    /// Thread/worker count: 2t, 4w, none.
    Cardinality,
    /// A registry-governed identifier token.
    Token,
    /// `+`-joined sub-values (e.g. contention=4t+1w).
    Compound,
}

/// A seeded condition definition.
pub struct ConditionDef {
    pub key: &'static str,
    pub ctype: ConditionType,
    pub values: &'static [&'static str],
    pub domains: &'static [&'static str],
}

/// The seeded condition registry.
pub const SEEDED_CONDITIONS: &[ConditionDef] = &[
    ConditionDef { key: "size",       ctype: ConditionType::Magnitude,  values: &[], domains: &["mem", "io", "crypto", "sync", "ipc", "storage", "reassemble", "pipeline"] },
    ConditionDef { key: "cache",      ctype: ConditionType::Enum,       values: &["l1", "l2", "l3", "dram"], domains: &["mem", "crypto"] },
    ConditionDef { key: "thermal",    ctype: ConditionType::Enum,       values: &["burst", "sustained"], domains: &["mem", "io", "sync", "crypto", "sched", "pool", "ipc"] },
    ConditionDef { key: "contention", ctype: ConditionType::Compound,   values: &["none", "mixed", "busy", "requests"], domains: &["sync", "pool", "sched", "ipc", "audit"] },
    ConditionDef { key: "state",      ctype: ConditionType::Enum,       values: &["cold", "warm", "saturated", "drained", "prefaulted", "sustained"], domains: &["mem", "io", "pool", "ipc"] },
    ConditionDef { key: "order",      ctype: ConditionType::Enum,       values: &["sequential", "random", "strided", "ooo_50pct"], domains: &["mem", "reassemble"] },
    ConditionDef { key: "alloc",      ctype: ConditionType::Enum,       values: &["none", "fresh", "reuse", "pool"], domains: &["crypto", "mem", "sync"] },
    ConditionDef { key: "zero",       ctype: ConditionType::Enum,       values: &["yes", "no"], domains: &["mem", "pool"] },
    ConditionDef { key: "variant",    ctype: ConditionType::Token,      values: &[], domains: &["crypto", "sync", "sched", "io", "ipc", "audit", "reassemble", "pipeline", "storage"] },
    ConditionDef { key: "ring",       ctype: ConditionType::Enum,       values: &["coop", "defer", "sqpoll", "bare"], domains: &["io"] },
    ConditionDef { key: "drain",      ctype: ConditionType::Enum,       values: &["fast", "slow", "blocked"], domains: &["io", "pool"] },
    ConditionDef { key: "workers",    ctype: ConditionType::Cardinality, values: &[], domains: &["sched", "crypto", "pipeline"] },
    ConditionDef { key: "res",        ctype: ConditionType::Token,      values: &[], domains: &["mem", "io", "sync", "crypto", "sched", "pool", "ipc"] },
    ConditionDef { key: "epoch",      ctype: ConditionType::Enum,       values: &["stable", "transition", "double"], domains: &["mem", "io", "sync", "crypto", "sched", "pool", "ipc"] },
    ConditionDef { key: "sndbuf",    ctype: ConditionType::Magnitude,  values: &[], domains: &["io"] },
    ConditionDef { key: "send",      ctype: ConditionType::Token,      values: &[], domains: &["io"] },
];

/// Look up a condition definition by key.
pub fn lookup(key: &str) -> Option<&'static ConditionDef> {
    SEEDED_CONDITIONS.iter().find(|c| c.key == key)
}

/// Validate a condition key+value against the registry and operation domain.
/// Returns `Ok(())` if valid, or a string describing the violation.
pub fn validate(key: &str, value: &str, op_domain: &str) -> Result<(), String> {
    let def = lookup(key).ok_or_else(|| format!("unregistered condition key '{key}'"))?;

    // Domain applicability
    if !def.domains.contains(&op_domain) {
        return Err(format!(
            "condition '{key}' not applicable to operation domain '{op_domain}' (valid: {:?})",
            def.domains,
        ));
    }

    // Value validation by type
    match def.ctype {
        ConditionType::Enum => {
            if !def.values.is_empty() && !def.values.contains(&value) {
                return Err(format!(
                    "invalid value '{value}' for enum condition '{key}' (valid: {:?})",
                    def.values,
                ));
            }
        }
        ConditionType::Magnitude => {
            validate_magnitude(value).map_err(|e| format!("condition '{key}': {e}"))?;
        }
        ConditionType::Cardinality => {
            validate_cardinality(value).map_err(|e| format!("condition '{key}': {e}"))?;
        }
        ConditionType::Compound => {
            // Sub-values are individually cardinality or the token "none"
            for sub in value.split('+') {
                if sub != "none" {
                    validate_cardinality(sub).map_err(|e| format!("condition '{key}' sub-value: {e}"))?;
                }
            }
        }
        ConditionType::Token => {
            // Token values are open (registry-governed). Validate syntax only.
            if value.is_empty() || !value.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                return Err(format!("invalid token value '{value}' for condition '{key}'"));
            }
        }
    }

    Ok(())
}

/// Canonicalize a `+`-joined compound value: sort sub-values by byte order.
pub fn canonicalize_compound(s: &str) -> String {
    let mut parts: Vec<&str> = s.split('+').collect();
    parts.sort();
    parts.dedup();
    parts.join("+")
}

/// Validate magnitude syntax: bare integer or integer + k/m/g suffix.
fn validate_magnitude(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("empty magnitude".to_owned());
    }
    let (digits, suffix) = split_magnitude(s);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid magnitude digits '{digits}'"));
    }
    match suffix {
        "" | "k" | "m" | "g" => Ok(()),
        other => Err(format!("invalid magnitude suffix '{other}' (valid: k, m, g)")),
    }
}

/// Validate cardinality syntax: "none" or digits + t/w suffix.
fn validate_cardinality(s: &str) -> Result<(), String> {
    if s == "none" {
        return Ok(());
    }
    if s.is_empty() {
        return Err("empty cardinality".to_owned());
    }
    let last = s.as_bytes()[s.len() - 1];
    if !matches!(last, b't' | b'w' | b'c' | b's') {
        return Err(format!("cardinality '{s}' must end with 't' (threads), 'w' (workers), 'c' (clients), or 's' (streams)"));
    }
    let digits = &s[..s.len() - 1];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid cardinality digits in '{s}'"));
    }
    if digits.starts_with('0') && digits.len() > 1 {
        return Err(format!("cardinality '{s}' has leading zeros"));
    }
    Ok(())
}

/// Split a magnitude string into (digits, suffix).
fn split_magnitude(s: &str) -> (&str, &str) {
    if s.is_empty() {
        return ("", "");
    }
    let last = s.as_bytes()[s.len() - 1];
    if last.is_ascii_lowercase() && !last.is_ascii_digit() {
        (&s[..s.len() - 1], &s[s.len() - 1..])
    } else {
        (s, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_finds_seeded() {
        assert!(lookup("size").is_some());
        assert!(lookup("cache").is_some());
        assert!(lookup("nonexistent").is_none());
    }

    #[test]
    fn validate_enum_accepts_valid() {
        assert!(validate("cache", "l1", "mem").is_ok());
        assert!(validate("cache", "dram", "crypto").is_ok());
    }

    #[test]
    fn validate_enum_rejects_invalid_value() {
        assert!(validate("cache", "l4", "mem").is_err());
    }

    #[test]
    fn validate_rejects_wrong_domain() {
        assert!(validate("cache", "l1", "io").is_err());
        assert!(validate("ring", "coop", "mem").is_err());
    }

    #[test]
    fn validate_magnitude_accepts() {
        assert!(validate("size", "64", "mem").is_ok());
        assert!(validate("size", "16k", "mem").is_ok());
        assert!(validate("size", "1m", "io").is_ok());
        assert!(validate("size", "2g", "crypto").is_ok());
    }

    #[test]
    fn validate_magnitude_rejects() {
        assert!(validate("size", "", "mem").is_err());
        assert!(validate("size", "k", "mem").is_err());
        assert!(validate("size", "64x", "mem").is_err());
    }

    #[test]
    fn validate_cardinality_accepts() {
        assert!(validate("contention", "none", "sync").is_ok());
        assert!(validate("contention", "8t", "sync").is_ok());
        assert!(validate("contention", "4t+1w", "sync").is_ok());
    }

    #[test]
    fn validate_cardinality_rejects_leading_zeros() {
        assert!(validate("workers", "04w", "sched").is_err());
    }

    #[test]
    fn canonicalize_compound_sorts() {
        assert_eq!(canonicalize_compound("4t+1w"), "1w+4t");
        assert_eq!(canonicalize_compound("8t"), "8t");
    }

    #[test]
    fn canonicalize_compound_deduplicates() {
        assert_eq!(canonicalize_compound("4t+4t"), "4t");
    }

    #[test]
    fn validate_token_accepts_lowercase_alphanumeric() {
        assert!(validate("variant", "aegis128l", "crypto").is_ok());
        assert!(validate("variant", "aes256gcm", "crypto").is_ok());
        assert!(validate("res", "cycles", "crypto").is_ok());
    }

    #[test]
    fn validate_token_rejects_empty() {
        assert!(validate("variant", "", "crypto").is_err());
    }
}
