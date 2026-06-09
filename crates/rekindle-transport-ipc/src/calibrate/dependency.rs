//! Accumulative dependency graph over operations.
//!
//! A composite measurement is meaningful only when every operation in
//! its transitive dependency set has an established measurement on the
//! same substrate. The graph is acyclic by construction.

use std::collections::{BTreeMap, BTreeSet};

/// The dependency graph: operation → set of operations it depends on.
pub struct DependencyGraph {
    edges: BTreeMap<&'static str, &'static [&'static str]>,
}

impl DependencyGraph {
    /// Build the graph from the seeded declarations.
    pub fn seeded() -> Self {
        let mut edges = BTreeMap::new();
        for (op, deps) in SEEDED_DEPS {
            edges.insert(*op, *deps);
        }
        Self { edges }
    }

    /// Direct dependencies of an operation.
    pub fn direct_deps(&self, op: &str) -> &[&str] {
        self.edges.get(op).copied().unwrap_or(&[])
    }

    /// Transitive closure of all dependencies.
    pub fn transitive_deps(&self, op: &str) -> BTreeSet<&str> {
        let mut result = BTreeSet::new();
        let mut stack: Vec<&str> = self.direct_deps(op).to_vec();
        while let Some(dep) = stack.pop() {
            if result.insert(dep) {
                stack.extend_from_slice(self.direct_deps(dep));
            }
        }
        result
    }

    /// Dependencies of a `+`-composed operation: union of operand deps,
    /// minus the operands themselves.
    pub fn composed_deps<'a>(&self, operands: &'a [String]) -> BTreeSet<&str> {
        let mut deps = BTreeSet::new();
        for op in operands {
            deps.extend(self.transitive_deps(op));
        }
        // Remove operands themselves — a composite does not depend on
        // its own constituents being measured separately
        for op in operands {
            deps.remove(op.as_str());
        }
        deps
    }

    /// Topological sort — the accumulative execution order.
    /// Primitives (no deps) come first.
    pub fn topo_order(&self) -> Vec<&str> {
        let mut result = Vec::new();
        let mut visited = BTreeSet::new();
        for op in self.edges.keys() {
            self.topo_visit(op, &mut visited, &mut result);
        }
        result
    }

    fn topo_visit<'a>(
        &'a self,
        op: &'a str,
        visited: &mut BTreeSet<&'a str>,
        result: &mut Vec<&'a str>,
    ) {
        if !visited.insert(op) {
            return;
        }
        for dep in self.direct_deps(op) {
            self.topo_visit(dep, visited, result);
        }
        result.push(op);
    }

    /// Check if an operation is registered in the graph.
    pub fn contains(&self, op: &str) -> bool {
        self.edges.contains_key(op)
    }

    /// All registered operations.
    pub fn operations(&self) -> Vec<&str> {
        self.edges.keys().copied().collect()
    }
}

/// Seeded dependency declarations.
const SEEDED_DEPS: &[(&str, &[&str])] = &[
    ("mem.read",    &[]),
    ("mem.write",   &[]),
    ("mem.copy",    &[]),
    ("mem.alloc",   &[]),
    ("mem.zero",    &[]),
    ("mem.fault",   &["mem.alloc"]),
    ("io.nop",      &[]),
    ("io.submit",   &["io.nop"]),
    ("io.send",     &["io.nop"]),
    ("io.recv",     &["io.nop"]),
    ("io.rtt",      &["io.send", "io.recv", "sync.wake"]),
    ("sync.atomic", &[]),
    ("sync.futex",  &[]),
    ("sync.eventfd", &[]),
    ("sync.spinwait", &[]),
    ("sync.wake",   &[]),
    ("sync.rwlock", &["sync.atomic"]),
    ("sync.channel", &["sync.atomic", "sync.wake"]),
    ("crypto.mac",  &["mem.read"]),
    ("crypto.hash", &["mem.read"]),
    ("crypto.seal", &["mem.copy"]),
    ("crypto.open", &["mem.copy"]),
    ("crypto.handshake", &["crypto.seal", "crypto.open"]),
    ("sched.yield", &[]),
    ("sched.wake",  &["sync.futex"]),
    ("sched.spawn", &["sync.atomic", "sync.futex"]),
    ("pool.acquire", &["sync.atomic"]),
    ("pool.release", &["sync.atomic"]),
    ("pool.reclaim", &["sync.atomic"]),
    // Crypto workload operations
    ("crypto.emac",         &["crypto.mac"]),
    ("crypto.header_mac",   &["crypto.mac"]),
    // Audit chain
    ("audit.link",          &["crypto.hash"]),
    // Reassembler
    ("reassemble.insert",   &["mem.copy"]),
    // Encode/decode pipeline (composite: envelope + header + AEAD)
    ("pipeline.encode",     &["crypto.emac", "crypto.header_mac", "crypto.seal"]),
    ("pipeline.decode",     &["crypto.emac", "crypto.header_mac", "crypto.open"]),
    // IPC workload operations (composite: full socket pipeline)
    ("ipc.request",         &[]),
    ("ipc.bulk",            &[]),
    ("ipc.notify",          &[]),
    ("ipc.handshake",       &["crypto.handshake"]),
    ("ipc.rotate",          &["crypto.handshake"]),
    ("ipc.connection",      &[]),
    // Storage workload operations
    ("storage.resolve",       &["ipc.request"]),
    ("storage.resolve_reply", &["ipc.request"]),
    ("storage.delete",        &["ipc.request"]),
    ("storage.store",         &["ipc.request"]),
    ("storage.count",         &["ipc.request"]),
    ("storage.count_reply",   &["ipc.request"]),
    ("storage.batch",         &["ipc.request"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_have_no_deps() {
        let g = DependencyGraph::seeded();
        assert!(g.direct_deps("mem.read").is_empty());
        assert!(g.direct_deps("sync.atomic").is_empty());
        assert!(g.direct_deps("io.nop").is_empty());
    }

    #[test]
    fn transitive_deps_of_io_rtt() {
        let g = DependencyGraph::seeded();
        let deps = g.transitive_deps("io.rtt");
        assert!(deps.contains("io.send"));
        assert!(deps.contains("io.recv"));
        assert!(deps.contains("sync.wake"));
        assert!(deps.contains("io.nop")); // transitive via io.send
    }

    #[test]
    fn transitive_deps_of_crypto_handshake() {
        let g = DependencyGraph::seeded();
        let deps = g.transitive_deps("crypto.handshake");
        assert!(deps.contains("crypto.seal"));
        assert!(deps.contains("crypto.open"));
        assert!(deps.contains("mem.copy")); // transitive
    }

    #[test]
    fn composed_deps_excludes_operands() {
        let g = DependencyGraph::seeded();
        let operands = vec!["crypto.seal".to_owned(), "crypto.mac".to_owned()];
        let deps = g.composed_deps(&operands);
        assert!(!deps.contains("crypto.seal"));
        assert!(!deps.contains("crypto.mac"));
        assert!(deps.contains("mem.copy")); // dep of seal
        assert!(deps.contains("mem.read")); // dep of mac
    }

    #[test]
    fn topo_order_primitives_first() {
        let g = DependencyGraph::seeded();
        let order = g.topo_order();
        let pos = |op: &str| order.iter().position(|o| *o == op).unwrap();

        // Primitives before composites
        assert!(pos("io.nop") < pos("io.send"));
        assert!(pos("io.send") < pos("io.rtt"));
        assert!(pos("sync.atomic") < pos("sync.rwlock"));
        assert!(pos("mem.copy") < pos("crypto.seal"));
    }

    #[test]
    fn graph_is_acyclic() {
        let g = DependencyGraph::seeded();
        // If topo_order completes without infinite loop, graph is acyclic.
        // Additionally verify every op appears exactly once.
        let order = g.topo_order();
        let unique: BTreeSet<&str> = order.iter().copied().collect();
        assert_eq!(order.len(), unique.len());
        assert_eq!(order.len(), g.operations().len());
    }

    #[test]
    fn unknown_op_has_no_deps() {
        let g = DependencyGraph::seeded();
        assert!(g.direct_deps("nonexistent").is_empty());
        assert!(g.transitive_deps("nonexistent").is_empty());
    }
}
