//! Deps trait surface for Phase 18 community lifecycle ops.
//!
//! One composite trait (`GovernanceRuntimeDeps`) abstracts every
//! src-tauri/Veilid/SQLite/Stronghold capability the crate's pure
//! orchestration logic needs. The adapter at
//! `src-tauri/src/services/governance_adapter.rs` implements it.
//!
//! Schwarzschild boundary: all Veilid types (`RecordKey`, `KeyPair`,
//! `RoutingContext`) are exchanged as opaque `String` / `Vec<u8>` here.
//! The crate never imports `veilid-core`.

mod roles;
mod runtime_deps;
mod types;

pub use roles::RoleTable;
pub use runtime_deps::GovernanceRuntimeDeps;
#[cfg(test)]
pub use runtime_deps::MockGovernanceRuntimeDeps;
pub use types::*;
