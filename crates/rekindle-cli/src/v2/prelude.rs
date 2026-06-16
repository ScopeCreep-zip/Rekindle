//! CLI prelude — single import for every command handler.
//!
//! Re-exports the cross-cutting types that every command handler,
//! TUI component, and streaming command needs. When the source crate
//! of any type changes, only this file updates — zero cascading
//! changes across command handlers.
//!
//! Usage: `use crate::v2::prelude::*;`

pub use rekindle_client::{AgentType, ChatRequest, DaemonClient, DaemonRequest, LifecycleRequest};
