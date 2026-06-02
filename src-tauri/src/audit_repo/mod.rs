//! Phase 4 — SQLite persistence for `audit_entries`.
//!
//! Two layers, one per submodule:
//! - [`store`] — **conn-level functions** (`insert_entry`, `load_all`,
//!   `load_since`, `load_tail`): pure `rusqlite` used inside
//!   `db_call`/`db_fire` closures.
//! - [`chain`] — **high-level helpers** (`append_async`, `verify_async`,
//!   `restore_chain`): pull the chain state from `AppState::audit_chain`,
//!   append + persist atomically, and broadcast `SystemEvent::AuditChainBroken`
//!   on verify failure.

mod chain;
mod store;

pub use chain::{append_async, restore_chain, verify_async, AuditVerifyResult};
pub use store::{insert_entry, load_all, load_since, load_tail};

#[cfg(test)]
mod tests;
