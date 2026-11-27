//! `TransportGuard` — RAII check at command entry.
//!
//! Mutating Tauri commands obtain a [`TransportGuard::write`] guard at
//! the top of their body. If the lifecycle isn't in a `can_write` state
//! (`Operational` or `Degraded`), construction returns
//! `LifecycleError::CannotWrite` and the command short-circuits.
//! Read-only commands use [`TransportGuard::read`].
//!
//! The guard itself does NOT mutate state on drop — its sole job is the
//! boundary check. Holding it for the duration of the command is purely
//! a lifetime-binding convenience so the borrow checker can verify the
//! check actually ran.

use crate::error::LifecycleError;
use crate::state::AppLifecycle;

/// Bound to an [`AppLifecycle`] for the duration of one command body.
/// Construction is the side-effecting step (the capability check); the
/// guard's presence at runtime is purely informational.
#[derive(Debug)]
pub struct TransportGuard<'a> {
    _lifecycle: &'a AppLifecycle,
}

impl<'a> TransportGuard<'a> {
    /// Acquire a write guard.
    ///
    /// # Errors
    /// Returns [`LifecycleError::CannotWrite`] if the lifecycle isn't in
    /// a state that accepts writes (i.e. not `Operational` or `Degraded`).
    pub fn write(lifecycle: &'a AppLifecycle) -> Result<Self, LifecycleError> {
        let s = lifecycle.state();
        if !s.can_write() {
            return Err(LifecycleError::CannotWrite { state: s });
        }
        Ok(Self {
            _lifecycle: lifecycle,
        })
    }

    /// Acquire a read guard.
    ///
    /// # Errors
    /// Returns [`LifecycleError::CannotQuery`] if the lifecycle isn't in
    /// a state that accepts reads (i.e. not `Operational`, `Degraded`,
    /// or `Detached`).
    pub fn read(lifecycle: &'a AppLifecycle) -> Result<Self, LifecycleError> {
        let s = lifecycle.state();
        if !s.can_query() {
            return Err(LifecycleError::CannotQuery { state: s });
        }
        Ok(Self {
            _lifecycle: lifecycle,
        })
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
