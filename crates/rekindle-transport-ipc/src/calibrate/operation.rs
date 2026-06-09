//! Operation paths — hierarchical dotted noun paths with `+` composition.
//!
//! An operation names the work being measured. The path is drawn from
//! the seeded registry. Unregistered paths are invalid.
//!
//! `+` composition joins two operations measured as one inseparable unit.
//! Operands are sorted and deduplicated in canonical form.

/// A parsed operation path, possibly `+`-composed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OperationPath {
    /// Sorted, deduplicated operand paths.
    /// A single operation has exactly one operand.
    operands: Vec<String>,
}

impl OperationPath {
    /// Parse an operation string. Handles `+` composition.
    /// Does NOT validate against registry — call `is_registered()` for that.
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }
        let mut operands: Vec<String> = s.split('+')
            .map(|op| op.to_ascii_lowercase())
            .filter(|op| !op.is_empty())
            .collect();
        if operands.is_empty() {
            return None;
        }
        // Validate syntax: each operand must be dotted labels of [a-z0-9]
        for op in &operands {
            if !is_valid_path_syntax(op) {
                return None;
            }
        }
        // Sort for canonical form
        operands.sort();
        // Deduplicate (idempotence: a+a → a)
        operands.dedup();
        Some(Self { operands })
    }

    /// The canonical string: sorted operands joined by `+`.
    pub fn canonical(&self) -> String {
        self.operands.join("+")
    }

    /// The domain — the first dotted label of the first operand.
    /// For composed operations, returns the first operand's domain.
    pub fn domain(&self) -> &str {
        self.operands.first()
            .and_then(|op| op.split('.').next())
            .unwrap_or("")
    }

    /// All unique domains across all operands.
    pub fn domains(&self) -> Vec<&str> {
        let mut domains: Vec<&str> = self.operands.iter()
            .filter_map(|op| op.split('.').next())
            .collect();
        domains.sort();
        domains.dedup();
        domains
    }

    /// Whether this is a `+`-composed operation.
    pub fn is_composed(&self) -> bool {
        self.operands.len() > 1
    }

    /// The individual operand paths.
    pub fn operands(&self) -> &[String] {
        &self.operands
    }

    /// Whether every operand is in the seeded registry.
    pub fn all_registered(&self) -> bool {
        self.operands.iter().all(|op| SEEDED_OPERATIONS.contains(&op.as_str()))
    }
}

/// Validate path syntax: one or more labels of `[a-z][a-z0-9_]*` joined by `.`
fn is_valid_path_syntax(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    for label in s.split('.') {
        if label.is_empty() {
            return false;
        }
        let mut chars = label.chars();
        match chars.next() {
            Some(c) if c.is_ascii_lowercase() => {}
            _ => return false,
        }
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return false;
        }
    }
    true
}

/// The seeded operation registry.
pub const SEEDED_OPERATIONS: &[&str] = &[
    // Memory primitives
    "mem.copy", "mem.read", "mem.write", "mem.alloc", "mem.zero", "mem.fault",
    // I/O primitives
    "io.nop", "io.send", "io.recv", "io.rtt", "io.submit",
    // Synchronization primitives
    "sync.atomic", "sync.futex", "sync.eventfd", "sync.spinwait",
    "sync.rwlock", "sync.channel", "sync.wake",
    // Crypto primitives
    "crypto.seal", "crypto.open", "crypto.mac", "crypto.hash", "crypto.handshake",
    "crypto.emac", "crypto.header_mac",
    // Scheduler primitives
    "sched.spawn", "sched.wake", "sched.yield",
    // Pool primitives
    "pool.acquire", "pool.release", "pool.reclaim",
    // Audit chain
    "audit.link",
    // Reassembler
    "reassemble.insert",
    // Encode/decode pipeline
    "pipeline.encode", "pipeline.decode",
    // IPC workload operations
    "ipc.request", "ipc.bulk", "ipc.notify", "ipc.handshake", "ipc.rotate",
    "ipc.connection",
    // Storage workload operations
    "storage.resolve", "storage.resolve_reply", "storage.delete",
    "storage.store", "storage.count", "storage.count_reply", "storage.batch",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let op = OperationPath::parse("mem.copy").unwrap();
        assert_eq!(op.canonical(), "mem.copy");
        assert!(!op.is_composed());
    }

    #[test]
    fn parse_composed_sorts() {
        let op = OperationPath::parse("crypto.seal+crypto.mac").unwrap();
        assert_eq!(op.canonical(), "crypto.mac+crypto.seal");
        assert!(op.is_composed());
    }

    #[test]
    fn composed_deduplicates() {
        let op = OperationPath::parse("crypto.seal+crypto.seal").unwrap();
        assert_eq!(op.canonical(), "crypto.seal");
        assert!(!op.is_composed());
    }

    #[test]
    fn domain_extraction() {
        let op = OperationPath::parse("crypto.seal").unwrap();
        assert_eq!(op.domain(), "crypto");
    }

    #[test]
    fn composed_domains() {
        let op = OperationPath::parse("crypto.seal+mem.copy").unwrap();
        assert_eq!(op.domains(), vec!["crypto", "mem"]);
    }

    #[test]
    fn rejects_empty() {
        assert!(OperationPath::parse("").is_none());
    }

    #[test]
    fn rejects_invalid_syntax() {
        assert!(OperationPath::parse("123.bad").is_none());
        assert!(OperationPath::parse(".leading_dot").is_none());
        assert!(OperationPath::parse("trailing.").is_none());
    }

    #[test]
    fn uppercase_input_is_lowercased() {
        // The parser normalizes case — uppercase input is accepted and lowercased.
        let op = OperationPath::parse("UPPER.case").unwrap();
        assert_eq!(op.canonical(), "upper.case");
    }

    #[test]
    fn all_seeded_are_valid_syntax() {
        for op in SEEDED_OPERATIONS {
            assert!(is_valid_path_syntax(op), "seeded operation '{op}' has invalid syntax");
        }
    }

    #[test]
    fn registered_check() {
        let op = OperationPath::parse("mem.copy").unwrap();
        assert!(op.all_registered());

        let op = OperationPath::parse("mem.nonexistent").unwrap();
        assert!(!op.all_registered());
    }
}
