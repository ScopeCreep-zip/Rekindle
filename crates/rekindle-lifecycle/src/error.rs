//! Lifecycle error taxonomy.

use crate::state::LifecycleState;

/// Errors raised by the lifecycle FSM and the [`crate::TransportGuard`].
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum LifecycleError {
    /// The requested state transition is not a valid edge in the FSM.
    /// E.g. attempting `Stopped → Operational` (must go through Starting
    /// then Locked then Resuming).
    #[error("invalid lifecycle transition {from:?} → {to:?}")]
    InvalidTransition {
        from: LifecycleState,
        to: LifecycleState,
    },
    /// A mutating command was attempted in a state where writes are not
    /// permitted (everything except `Operational` and `Degraded`).
    #[error("cannot write in state {state:?}")]
    CannotWrite { state: LifecycleState },
    /// A read-only command was attempted in a state where queries are not
    /// permitted (everything except `Operational`, `Degraded`, `Detached`).
    #[error("cannot read in state {state:?}")]
    CannotQuery { state: LifecycleState },
}
