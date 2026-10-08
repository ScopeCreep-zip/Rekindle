//! Persistent state management.
//!
//! Bridges between the IPC layer (which knows nothing about Veilid) and the
//! rekindle-transport crate (which owns all Veilid operations). This module
//! manages:
//!
//! - Session loading/saving (session.json, atomic write); the directories
//!   come from `rekindle_db::paths::DataRoot`.
//!
//! **Boundary**: This module imports `rekindle_transport::Session` and
//! `rekindle_transport::TransportConfig`. All network operations are
//! delegated to rekindle-transport's broadcast/ and subscriptions/ modules.

pub mod audit;
pub mod keystore;

use std::path::Path;

use rekindle_transport::Session;

/// Load session from disk if it exists.
pub fn load_session(path: &Path) -> anyhow::Result<Option<Session>> {
    match Session::load(path) {
        Ok(Some(session)) => Ok(Some(session)),
        Ok(None) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("failed to load session: {e}")),
    }
}

/// Save session to disk via atomic write.
pub fn save_session(session: &Session, path: &Path) -> anyhow::Result<()> {
    session
        .save(path)
        .map_err(|e| anyhow::anyhow!("failed to save session: {e}"))
}
