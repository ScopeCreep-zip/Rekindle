//! Phase 18.h.2 — Plate Gate thin facade.
//!
//! The 6 public segment operations + `MAX_SEGMENTS` constant live in
//! `rekindle_governance_runtime::segments`. This module constructs a
//! `GovernanceAdapter` per call and delegates.

use std::sync::Arc;

use rekindle_governance_runtime as gov_rt;
use tauri::Manager;

use crate::state::SharedState;

#[allow(unused_imports, reason = "MAX_SEGMENTS re-exported for any external admin tooling")]
pub use gov_rt::{SegmentDescriptor, MAX_SEGMENTS};

fn build_adapter(
    state: &SharedState,
) -> Result<crate::services::governance_adapter::GovernanceAdapter, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<crate::db::DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    Ok(crate::services::governance_adapter::GovernanceAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    ))
}

pub fn segment_descriptors(state: &SharedState, community_id: &str) -> Vec<SegmentDescriptor> {
    let Ok(adapter) = build_adapter(state) else {
        return Vec::new();
    };
    gov_rt::segment_descriptors(&adapter, community_id)
}

pub async fn highest_segment_full(
    state: &SharedState,
    community_id: &str,
) -> Result<bool, String> {
    let adapter = build_adapter(state)?;
    gov_rt::highest_segment_full(&adapter, community_id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn expand_community_segment(
    state: &SharedState,
    community_id: &str,
) -> Result<u32, String> {
    let adapter = build_adapter(state)?;
    gov_rt::expand_community_segment(&adapter, community_id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn open_new_segments(state: &SharedState, community_id: &str) {
    let Ok(adapter) = build_adapter(state) else {
        return;
    };
    gov_rt::open_new_segments(&adapter, community_id).await;
}

pub async fn ensure_channel_segment_record(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<String, String> {
    let adapter = build_adapter(state)?;
    gov_rt::ensure_channel_segment_record(&adapter, community_id, channel_id)
        .await
        .map_err(|e| e.to_string())
}

pub fn channel_record_keys_per_segment(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Vec<(u32, String)> {
    let Ok(adapter) = build_adapter(state) else {
        return Vec::new();
    };
    gov_rt::channel_record_keys_per_segment(&adapter, community_id, channel_id)
}
