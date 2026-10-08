//! The desktop's audit-chain helpers over `rekindle_db::repo::audit`: pull
//! the chain state from `AppState::audit_chain`, append and persist, and
//! broadcast `SystemEvent::AuditChainBroken` when verify fails.

mod chain;

pub use chain::{append_async, restore_chain, verify_async, AuditVerifyResult};
