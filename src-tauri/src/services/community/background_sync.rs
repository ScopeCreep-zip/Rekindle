//! Architecture §17.3 Tier 2 — on-demand background fetch for mobile
//! and desktop suspend/resume.
//!
//! When the OS hands us a brief window of background execution we
//! cannot afford to wait for the 60-second `inspect` interval. This
//! module exposes a single `run_background_sync_now` entry point that
//! iterates every joined community, runs one `inspect_dht_record`
//! sweep per tracked record (governance + registry + every channel),
//! fetches any new subkeys, and lets the existing message handlers
//! emit notifications. Returns a summary so the caller can surface
//! "checked N communities, found M new messages".

use std::sync::Arc;

use serde::Serialize;

use super::inspect::{inspect_record, tracked_record_keys};
use crate::state::AppState;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundSyncReport {
    pub communities_checked: u32,
    pub records_inspected: u32,
    pub failed_records: u32,
    pub elapsed_ms: u32,
}

pub async fn run_background_sync_now(
    state: &Arc<AppState>,
) -> Result<BackgroundSyncReport, String> {
    let started = std::time::Instant::now();
    let community_ids: Vec<String> = state
        .communities
        .read()
        .keys()
        .cloned()
        .collect();
    let mut report = BackgroundSyncReport::default();
    for community_id in community_ids {
        report.communities_checked = report.communities_checked.saturating_add(1);
        let Some(records) = tracked_record_keys(state, &community_id) else {
            continue;
        };
        for record_key in records {
            report.records_inspected = report.records_inspected.saturating_add(1);
            if let Err(e) = inspect_record(state, &community_id, &record_key).await {
                report.failed_records = report.failed_records.saturating_add(1);
                tracing::debug!(
                    community = %community_id,
                    record_key = %record_key,
                    error = %e,
                    "background sync inspect failed"
                );
            }
        }
    }
    report.elapsed_ms =
        u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    Ok(report)
}
