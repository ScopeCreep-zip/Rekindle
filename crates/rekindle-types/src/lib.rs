//! Shared type definitions for the Rekindle v2.0 community system.
//!
//! Tier 1 vocabulary crate — every other Rekindle crate depends on this.
//! Contains zero logic, zero I/O, zero async. Only data definitions.
//!
//! These are the v2.0 types for flat SMPL governance. They do NOT re-export
//! v1.0 types from rekindle-protocol — those are replaced, not wrapped.

pub mod channel;
pub mod error;
pub mod governance;
pub mod id;
pub mod invite;
pub mod permissions;
pub mod presence;
