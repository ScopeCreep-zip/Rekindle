//! Registry management — extensible vocabulary validation.
//!
//! The seeded registries live in their respective modules (quantity.rs,
//! operation.rs, condition.rs). This module provides the extension
//! validation machinery: ensuring additions don't collide, don't
//! introduce dependency cycles, and don't violate domain constraints.

/// Status of a registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Deprecated,
    Reserved,
}

/// A registry entry descriptor — used for extension validation.
#[derive(Debug, Clone)]
pub struct Entry {
    pub registry: &'static str,
    pub token: &'static str,
    pub semantics: &'static str,
    pub since: &'static str,
    pub status: Status,
}

/// Validate that a proposed operation addition doesn't collide with
/// existing entries and doesn't create a dependency cycle.
pub fn validate_operation_extension(
    token: &str,
    deps: &[&str],
    existing_ops: &[&str],
    existing_deps: &[(&str, &[&str])],
) -> Result<(), String> {
    // No collision
    if existing_ops.contains(&token) {
        return Err(format!("operation '{token}' already registered"));
    }
    // All deps must exist
    for dep in deps {
        if !existing_ops.contains(dep) {
            return Err(format!("dependency '{dep}' of '{token}' is not registered"));
        }
    }
    // No cycle: check if any dep transitively reaches `token`.
    // Since `token` is new and not yet in the graph, a cycle would
    // require one of its deps to depend on `token` — which is impossible
    // since `token` isn't registered yet. But we check the full
    // transitive closure for completeness.
    for dep in deps {
        if would_reach(dep, token, existing_deps) {
            return Err(format!("adding '{token}' with dep '{dep}' creates a cycle"));
        }
    }
    Ok(())
}

/// Validate a proposed condition key addition.
pub fn validate_condition_extension(
    key: &str,
    existing_keys: &[&str],
) -> Result<(), String> {
    if existing_keys.contains(&key) {
        return Err(format!("condition key '{key}' already registered"));
    }
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return Err(format!("invalid condition key syntax: '{key}'"));
    }
    Ok(())
}

fn would_reach(from: &str, target: &str, deps: &[(&str, &[&str])]) -> bool {
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![from];
    while let Some(current) = stack.pop() {
        if current == target {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some((_, d)) = deps.iter().find(|(op, _)| *op == current) {
            stack.extend(d.iter());
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibrate::operation::SEEDED_OPERATIONS;

    #[test]
    fn rejects_collision() {
        let result = validate_operation_extension("mem.copy", &[], SEEDED_OPERATIONS, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn accepts_novel() {
        let result = validate_operation_extension("pci.dma", &[], SEEDED_OPERATIONS, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_missing_dep() {
        let result = validate_operation_extension("net.tcp", &["net.socket"], SEEDED_OPERATIONS, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn condition_rejects_collision() {
        let existing = vec!["size", "cache"];
        assert!(validate_condition_extension("size", &existing).is_err());
    }

    #[test]
    fn condition_accepts_novel() {
        let existing = vec!["size", "cache"];
        assert!(validate_condition_extension("numa", &existing).is_ok());
    }
}
