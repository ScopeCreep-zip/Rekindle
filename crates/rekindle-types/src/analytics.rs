//! Architecture §24.1 — local-only community analytics types.
//!
//! All values are aggregates computed from local SQLite. The protocol
//! never exchanges these — analytics data never leaves the device.

use serde::{Deserialize, Serialize};

/// One day-bucketed sample. `day_unix_ms` is midnight UTC of the
/// bucket; `value` is the metric's value for that 24-hour window.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailySample {
    pub day_unix_ms: i64,
    pub value: u32,
}

/// Daily timeseries — the last N days of a metric, oldest first.
/// Architecture §32 Phase 7 Week 23 ("messages per channel per day",
/// "active member count per day") — analytics dashboards prefer the
/// timeseries for sparkline rendering, while the rolling aggregates
/// (`*_7d` etc.) stay alongside as convenience values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyTimeseries {
    pub samples: Vec<DailySample>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberMetrics {
    pub total_members: u32,
    pub active_7d: u32,
    pub active_30d: u32,
    pub joins_7d: u32,
    pub leaves_7d: u32,
    /// Fraction in `[0.0, 1.0]` — members who joined 30+ days ago and
    /// remained active in the last 7 days. `f64` to skip the clippy
    /// precision-loss complaint without an `#[allow]`.
    pub retention_7_of_30: f64,
    /// Architecture §32 Week 23 — distinct active member count per day
    /// for the last 30 days.
    pub active_per_day: DailyTimeseries,
    /// Joins per day for the last 30 days.
    pub joins_per_day: DailyTimeseries,
    /// Leaves per day for the last 30 days.
    pub leaves_per_day: DailyTimeseries,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMetrics {
    pub channel_id: String,
    pub messages_7d: u32,
    pub unique_posters_7d: u32,
    pub peak_concurrent_voice: u32,
    /// Architecture §32 Week 23 — message count per day for the last
    /// 30 days, oldest first.
    pub messages_per_day: DailyTimeseries,
    /// Distinct posters per day for the last 30 days.
    pub unique_posters_per_day: DailyTimeseries,
}

/// One sample point — `day_unix_ms` is midnight UTC of that day,
/// `member_count` is the cumulative member count at that time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthSample {
    pub day_unix_ms: i64,
    pub member_count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthMetrics {
    pub samples: Vec<GrowthSample>,
}

/// Architecture §32 Phase 7 Week 23 — "peak activity hours". Bucketed
/// histogram over the 24 hours of UTC day. `hour_counts[h]` is the
/// number of messages whose hour-of-day equals `h`, summed across the
/// last 30 days (the same window as the daily timeseries) so the
/// distribution is statistically meaningful.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityByHour {
    /// One u32 per UTC hour 0..=23.
    pub hour_counts: [u32; 24],
}

impl ActivityByHour {
    /// Index of the bucket with the largest count (ties broken by
    /// lowest hour). Returns 0 for an empty histogram — UI should
    /// suppress display when every bucket is 0.
    #[must_use]
    pub fn peak_hour(&self) -> u8 {
        let mut best_hour: u8 = 0;
        let mut best_count: u32 = 0;
        for (hour, count) in self.hour_counts.iter().enumerate() {
            if *count > best_count {
                best_count = *count;
                best_hour = u8::try_from(hour).unwrap_or(0);
            }
        }
        best_hour
    }
}

/// Architecture §32 Phase 7 Week 23 — "storage usage per community"
/// breakdown. All values are bytes; `total_bytes` is the sum and the
/// component fields let the UI render a stacked-bar drill-down. Values
/// are advisory approximations from `LENGTH(...)` plus a fixed per-row
/// overhead, not exact disk page counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub total_bytes: u64,
    pub message_bytes: u64,
    pub thread_message_bytes: u64,
    pub channel_pin_bytes: u64,
    pub read_state_bytes: u64,
    pub voice_event_bytes: u64,
    pub member_leave_bytes: u64,
    pub metadata_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityAnalytics {
    pub community_id: String,
    pub members: MemberMetrics,
    pub channels: Vec<ChannelMetrics>,
    pub growth: GrowthMetrics,
    pub activity_by_hour: ActivityByHour,
    pub storage_usage: StorageUsage,
    /// Compute time in milliseconds (so the UI can show "queried in 12ms").
    pub computed_in_ms: u32,
}
