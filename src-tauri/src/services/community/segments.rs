//! Phase 18.h.2 — Plate Gate thin facade.
//!
//! The 6 public segment operations + `MAX_SEGMENTS` constant live in
//! `rekindle_governance_runtime::segments`. This module constructs a
//! call over the Host's `HostGovernance` (plan E3.1 B2f).

use rekindle_governance_runtime as gov_rt;

use crate::state::SharedState;

use gov_rt::SegmentDescriptor;

fn build_adapter(
    state: &SharedState,
) -> Result<rekindle_node::host::adapters::governance::HostGovernance, String> {
    Ok(crate::state_helpers::host_governance(state)?)
}

pub fn segment_descriptors(state: &SharedState, community_id: &str) -> Vec<SegmentDescriptor> {
    let Ok(adapter) = build_adapter(state) else {
        return Vec::new();
    };
    gov_rt::segment_descriptors(&adapter, community_id)
}

pub async fn highest_segment_full(state: &SharedState, community_id: &str) -> Result<bool, String> {
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
