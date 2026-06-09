//! Physics-derived validation predicates over measurement sets.
//!
//! Each rule is a machine-checkable bound. A measurement that violates
//! a physics bound is flagged untrusted — it is still emitted (the
//! dataset records failures as data), with a diagnosis.

use std::collections::BTreeMap;

/// Severity of a validation rule violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The measurement is physically impossible. Flag untrusted.
    Error,
    /// The measurement is suspicious but physically possible.
    Warn,
}

/// Result of evaluating one validation rule against one measurement.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub rule_id: &'static str,
    pub predicate: String,
    pub passed: bool,
    pub severity: Severity,
    pub observed: f64,
    pub bound: f64,
    pub diagnosis: Option<String>,
}

/// A seeded validation rule.
struct Rule {
    id: &'static str,
    severity: Severity,
    /// The subject pattern (canonical prefix match on the OSC id).
    subject_op: &'static str,
    /// The bound operation to compare against.
    bound_op: &'static str,
    /// The relation: true if the measurement is valid.
    /// (subject_value, bound_value) -> bool
    check: fn(f64, f64) -> bool,
    /// Multiplier on the bound value (e.g. 2.0 for rtt >= 2*send).
    bound_multiplier: f64,
    diagnosis: &'static str,
}

const SEEDED_RULES: &[Rule] = &[
    Rule {
        id: "copy_le_read",
        severity: Severity::Error,
        subject_op: "mem.copy",
        bound_op: "mem.read",
        check: |subject, bound| subject <= bound,
        bound_multiplier: 1.0,
        diagnosis: "copy bandwidth exceeds read bandwidth; the copy was likely \
                    elided by the compiler, or mem.read has a sequential \
                    dependency chain bottleneck",
    },
    Rule {
        id: "seal_le_copy",
        severity: Severity::Error,
        subject_op: "crypto.seal",
        bound_op: "mem.copy",
        check: |subject, bound| subject <= bound,
        bound_multiplier: 1.0,
        diagnosis: "seal_into bandwidth exceeds memcpy; seal does at least \
                    one copy of work plus compute, so it cannot exceed copy",
    },
    Rule {
        id: "sealalloc_le_sealinto",
        severity: Severity::Error,
        subject_op: "crypto.seal",
        bound_op: "crypto.seal",
        check: |subject, bound| subject <= bound,
        bound_multiplier: 1.0,
        diagnosis: "allocating seal exceeds in-place seal; allocating does \
                    strictly more work",
    },
    Rule {
        id: "rtt_ge_2x_send",
        severity: Severity::Error,
        subject_op: "io.rtt",
        bound_op: "io.send",
        check: |subject, bound| subject >= bound,
        bound_multiplier: 2.0,
        diagnosis: "round-trip latency less than twice one-way send; \
                    physically impossible for a real round trip",
    },
    Rule {
        id: "contended_ge_uncontended",
        severity: Severity::Error,
        subject_op: "sync.atomic",
        bound_op: "sync.atomic",
        check: |subject, bound| subject >= bound,
        bound_multiplier: 1.0,
        diagnosis: "contended atomic faster than uncontended; the contended \
                    case is likely not actually contending",
    },
    Rule {
        id: "pool_contended_monotone",
        severity: Severity::Warn,
        subject_op: "pool.acquire",
        bound_op: "pool.acquire",
        check: |subject, bound| subject >= bound,
        bound_multiplier: 1.0,
        diagnosis: "pool acquire latency does not rise with contention; \
                    either lock-free (expected) or contention not generated",
    },
];

/// Evaluate all applicable seeded rules for a given measurement.
///
/// `subject_id` is the canonical OSC identifier of the measurement.
/// `subject_value` is the measured value.
/// `established` maps canonical OSC identifiers to their measured values.
///
/// Returns results for every rule whose subject AND bound operands
/// are both present in the measurement set.
pub fn evaluate(
    subject_id: &str,
    subject_value: f64,
    established: &BTreeMap<String, f64>,
) -> Vec<ValidationResult> {
    let mut results = Vec::new();

    for rule in SEEDED_RULES {
        // Check if the subject matches this rule's operation
        if !subject_id.contains(&format!("/{}", rule.subject_op)) {
            continue;
        }

        // Find a matching bound measurement.
        // For same-op rules (e.g. contended vs uncontended), the bound
        // is a DIFFERENT measurement of the same operation with different
        // conditions. For cross-op rules, the bound is a different operation.
        let bound_value = if rule.subject_op == rule.bound_op {
            // Same-op rule: find any established measurement of the same
            // operation that is NOT the subject itself.
            established.iter()
                .find(|(id, _)| {
                    *id != subject_id && id.contains(&format!("/{}", rule.bound_op))
                })
                .map(|(_, v)| *v)
        } else {
            // Cross-op rule: find any established measurement of the bound operation
            // with matching size condition (if present).
            let subject_size = extract_condition(subject_id, "size");
            established.iter()
                .find(|(id, _)| {
                    id.contains(&format!("/{}", rule.bound_op))
                        && match &subject_size {
                            Some(s) => extract_condition(id, "size").as_deref() == Some(s),
                            None => true,
                        }
                })
                .map(|(_, v)| *v)
        };

        let bound_value = match bound_value {
            Some(v) => v * rule.bound_multiplier,
            None => continue, // Bound not established — rule not applicable
        };

        let passed = (rule.check)(subject_value, bound_value);

        results.push(ValidationResult {
            rule_id: rule.id,
            predicate: format!(
                "{} {} {} * {:.2}",
                rule.subject_op,
                if (rule.check)(1.0, 0.0) { ">=" } else { "<=" },
                rule.bound_op,
                rule.bound_multiplier,
            ),
            passed,
            severity: rule.severity,
            observed: subject_value,
            bound: bound_value,
            diagnosis: if passed { None } else { Some(rule.diagnosis.to_owned()) },
        });
    }

    results
}

/// Extract a condition value from a canonical OSC identifier string.
fn extract_condition<'a>(id: &'a str, key: &str) -> Option<&'a str> {
    let conditions = id.split('?').nth(1)?;
    let conditions = conditions.split('@').next().unwrap_or(conditions);
    for pair in conditions.split('&') {
        if let Some(val) = pair.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
            return Some(val);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_le_read_passes_when_valid() {
        let mut established = BTreeMap::new();
        established.insert("osc:bw/mem.read?size=64m".to_owned(), 19.0);
        established.insert("osc:bw/mem.copy?size=64m".to_owned(), 15.0);

        let results = evaluate("osc:bw/mem.copy?size=64m", 15.0, &established);
        assert!(!results.is_empty());
        assert!(results[0].passed);
    }

    #[test]
    fn copy_le_read_fails_when_copy_exceeds_read() {
        let mut established = BTreeMap::new();
        established.insert("osc:bw/mem.read?size=64m".to_owned(), 19.0);

        let results = evaluate("osc:bw/mem.copy?size=64m", 25.0, &established);
        assert!(!results.is_empty());
        assert!(!results[0].passed);
        assert_eq!(results[0].severity, Severity::Error);
        assert!(results[0].diagnosis.is_some());
    }

    #[test]
    fn rtt_ge_2x_send_passes() {
        let mut established = BTreeMap::new();
        established.insert("osc:lat/io.send?size=64".to_owned(), 3.0);

        let results = evaluate("osc:lat/io.rtt?size=64", 7.0, &established);
        let rtt_rule = results.iter().find(|r| r.rule_id == "rtt_ge_2x_send");
        assert!(rtt_rule.is_some());
        assert!(rtt_rule.unwrap().passed);
    }

    #[test]
    fn rtt_ge_2x_send_fails() {
        let mut established = BTreeMap::new();
        established.insert("osc:lat/io.send?size=64".to_owned(), 5.0);

        let results = evaluate("osc:lat/io.rtt?size=64", 8.0, &established);
        let rtt_rule = results.iter().find(|r| r.rule_id == "rtt_ge_2x_send");
        assert!(rtt_rule.is_some());
        assert!(!rtt_rule.unwrap().passed);
    }

    #[test]
    fn no_results_when_bound_not_established() {
        let established = BTreeMap::new();
        let results = evaluate("osc:bw/mem.copy?size=64m", 15.0, &established);
        assert!(results.is_empty());
    }

    #[test]
    fn extract_condition_works() {
        assert_eq!(extract_condition("osc:bw/mem.copy?cache=l1&size=64m", "size"), Some("64m"));
        assert_eq!(extract_condition("osc:bw/mem.copy?cache=l1&size=64m", "cache"), Some("l1"));
        assert_eq!(extract_condition("osc:bw/mem.copy?cache=l1&size=64m@x86_64.linux.uring", "size"), Some("64m"));
        assert_eq!(extract_condition("osc:bw/mem.copy", "size"), None);
    }
}
