//! User-facing transport configuration.
//!
//! All tunables that affect privacy, performance, and reliability are
//! centralized here. Consumers construct a [`TransportConfig`] and pass
//! it to [`TransportNode::start`](crate::broadcast::node::TransportNode::start).
//!
//! Type definitions live in `rekindle-types::config` (the single source of
//! truth for IPC-boundary types). This module re-exports them so that
//! existing `rekindle_transport::config::*` paths continue to work.

pub use rekindle_types::config::{
    SafetyConfig, SafetyProfile, SequencingPreference, StabilityPreference, TransportConfig,
};
