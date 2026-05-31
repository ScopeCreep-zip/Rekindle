#![forbid(unsafe_code)]
//! 9-state application lifecycle FSM hoisted from `rekindle-node::daemon`.
//!
//! The FSM tracks where the application is in its boot/login/shutdown
//! cycle. Capability gates (`can_query`, `can_write`, `can_unlock`)
//! advertise which commands are safe to run; mutating commands wrap
//! their body in [`TransportGuard::write`] to reject calls in states
//! where the side effect can't be safely produced (e.g. sending a DM
//! while the network is `Detached`).
//!
//! Phase 5 of the decomposed-harvest plan hoists the daemon's existing
//! 9-state FSM into this shared crate so the Tauri shell can use the
//! same machinery without taking a dependency on `rekindle-node`. The
//! transition table is verbatim from the daemon — no behavior change.
//!
//! Plan reference: `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 5.

pub mod error;
pub mod guard;
pub mod state;

pub use error::LifecycleError;
pub use guard::TransportGuard;
pub use state::{AppLifecycle, LifecycleState};
