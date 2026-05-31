//! Architecture §24.1 — local-only community analytics.
//!
//! All metrics are pure SQL aggregations against tables the device
//! already owns; nothing leaves the device. Every entry point takes a
//! `rusqlite::Connection` + `owner_key` + `community_id` — no AppState,
//! no Tauri, no Veilid. The src-tauri facade
//! (`services/community/analytics`) wraps these with `DbPool` +
//! permission gating.
//!
//! Tier 3 — depends only on `rekindle-types` (DTO definitions for
//! `CommunityAnalytics`, `MemberMetrics`, etc.) + `rusqlite`.

#![forbid(unsafe_code)]

pub mod activity_by_hour;
pub mod buckets;
pub mod channel_metrics;
pub mod growth;
pub mod member_metrics;
pub mod storage;

#[cfg(test)]
mod tests;

pub const SEVEN_DAYS_MS: i64 = 7 * 24 * 60 * 60 * 1000;
pub const THIRTY_DAYS_MS: i64 = 30 * 24 * 60 * 60 * 1000;
pub const ONE_DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Number of daily buckets shipped in every timeseries — matches the
/// 30-day growth sample window so the UI can render all metrics on
/// the same x-axis.
pub const DAILY_TIMESERIES_DAYS: u32 = 30;
