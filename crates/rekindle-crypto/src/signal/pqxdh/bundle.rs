//! Re-export of [`crate::signal::PreKeyBundle`] for the PQXDH module.
//!
//! Phase 3b unified the post-quantum bundle shape with the wire-format
//! `PreKeyBundle` so there's only one struct definition in
//! `crate::signal::prekeys`. This module re-exports it under the old
//! `PqPreKeyBundle` name to keep imports inside the pqxdh tree stable
//! during the wire-integration migration.

pub use crate::signal::prekeys::PreKeyBundle as PqPreKeyBundle;
