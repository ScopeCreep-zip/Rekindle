//! Support types of the loss controller (str0m `src/bwe/loss_controller.rs`,
//! moved into this file to stay under the workspace's 600-line cap; fields are
//! `pub(super)` so the controller can keep reading them).

use std::cmp::{max, min};
use std::time::{Duration, Instant};

use super::super::time::BweTimestamp;
use super::PacketResult;
use crate::TwccSendRecord;
use crate::{Bitrate, DataSize};

pub(super) struct Config {
    pub(super) observation_window_size: usize, // minimum is 2
    pub(super) observation_duration_lower_bound: Duration,
    pub(super) trendline_integration_enabled: bool,
    pub(super) temporal_weight_factor: f64,
    pub(super) instant_upper_bound_temporal_weight_factor: f64,
    pub(super) instant_upper_bound_loss_offset: f64,
    pub(super) instant_upper_bound_bandwidth_balance: Bitrate,
    pub(super) high_loss_rate_threshold: f64,
    pub(super) slope_of_bwe_high_loss_function: Bitrate,
    pub(super) bandwidth_cap_at_high_loss_rate: Bitrate,
    pub(super) initial_inherent_loss_estimate: f64,
    pub(super) inherent_loss_upper_bound_offset: f64,
    pub(super) inherent_loss_upper_bound_bandwidth_balance: Bitrate,
    pub(super) inherent_loss_lower_bound: f64,
    pub(super) newton_iterations: usize,
    pub(super) newton_step_size: f64,
    pub(super) not_increase_if_inherent_loss_less_than_average_loss: bool,
    pub(super) delayed_increase_window: Duration,
    pub(super) bandwidth_rampup_upper_bound_factor: f64,
    pub(super) candidate_factor: [f64; 3],
    pub(super) append_acknowledged_rate_candidate: bool,
    pub(super) append_delay_based_estimate_candidate: bool,
    pub(super) bandwidth_backoff_lower_bound_factor: f64,
    pub(super) rampup_acceleration_maxout_time: Duration,
    pub(super) rampup_acceleration_max_factor: Duration,
    pub(super) higher_bandwidth_bias_factor: f64,
    pub(super) higher_log_bandwidth_bias_factor: f64,
    pub(super) threshold_of_high_bandwidth_preference: f64,
    pub(super) bandwidth_preference_smoothing_factor: f64,
    pub(super) use_byte_loss_ratio: bool,
    pub(super) hold_duration_factor: f64,
    pub(super) bandwidth_rampup_hold_threshold: f64,
    pub(super) bandwidth_rampup_upper_bound_factor_in_hold: f64,
}

#[derive(Debug)]
pub(super) struct PacketResultsSummary {
    pub(super) num_packets: u64,
    pub(super) num_lost_packets: u64,
    pub(super) total_size: DataSize,
    pub(super) lost_size: DataSize,
    pub(super) first_send_time: Instant,
    pub(super) last_send_time: Instant,
}

impl PacketResultsSummary {
    pub fn new(first_send_time: Instant, last_send_time: Instant) -> PacketResultsSummary {
        PacketResultsSummary {
            num_packets: 0,
            num_lost_packets: 0,
            total_size: DataSize::ZERO,
            lost_size: DataSize::ZERO,
            last_send_time,
            first_send_time,
        }
    }

    pub fn from(records: &[impl PacketResult]) -> Option<PacketResultsSummary> {
        let first = records.first()?;

        let mut summary =
            PacketResultsSummary::new(first.local_send_time(), first.local_send_time());
        for record in records {
            let lost: u64 = record.lost().into();
            let size = record.size();

            summary.num_packets += 1;
            summary.total_size += size;
            summary.lost_size += size * lost;
            summary.num_lost_packets += lost;
            summary.first_send_time = min(summary.first_send_time, record.local_send_time());
            summary.last_send_time = max(summary.last_send_time, record.local_send_time());
        }

        Some(summary)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Observation {
    pub(super) num_packets: u64,
    pub(super) size: DataSize,
    pub(super) num_lost_packets: u64,
    pub(super) lost_size: DataSize,
    pub(super) num_received_packets: u64,
    pub(super) sending_rate: Bitrate,
    pub(super) id: u64,
    pub(super) is_initialized: bool,
}

impl Observation {
    pub const DUMMY: Self = Self {
        num_packets: 0,
        size: DataSize::ZERO,
        num_lost_packets: 0,
        lost_size: DataSize::ZERO,
        num_received_packets: 0,
        sending_rate: Bitrate::NEG_INFINITY,
        id: 0,
        is_initialized: false,
    };
}

pub(super) struct PartialObservation {
    pub(super) num_packets: u64,
    pub(super) num_lost_packets: u64,
    pub(super) size: DataSize,
    pub(super) lost_size: DataSize,
}

impl PartialObservation {
    pub fn new() -> PartialObservation {
        PartialObservation {
            num_packets: 0,
            num_lost_packets: 0,
            size: DataSize::ZERO,
            lost_size: DataSize::ZERO,
        }
    }

    pub fn update(&mut self, summary: &PacketResultsSummary) {
        self.num_packets += summary.num_packets;
        self.num_lost_packets += summary.num_lost_packets;
        self.size += summary.total_size;
        self.lost_size += summary.lost_size;
    }
}

/// An estimate derived from some candidate.
#[derive(Debug, Clone, Copy)]
pub(super) struct ChannelParameters {
    /// The estimated inherent loss
    pub(super) inherent_loss: f64,
    /// The estimated bandwidth
    pub(super) loss_limited_bandwidth: Bitrate,
}

impl ChannelParameters {
    pub fn new(inherent_loss: f64) -> ChannelParameters {
        ChannelParameters {
            inherent_loss,
            loss_limited_bandwidth: Bitrate::NEG_INFINITY,
        }
    }
}

/// HOLD mechanism info - prevents immediate ramp-up after loss detection
#[derive(Debug, Clone, Copy)]
pub(super) struct HoldInfo {
    pub(super) timestamp: BweTimestamp,
    pub(super) duration: Duration,
    pub(super) rate: Bitrate,
}

impl Default for HoldInfo {
    fn default() -> Self {
        const INIT_HOLD_DURATION: Duration = Duration::from_millis(300);
        Self {
            timestamp: BweTimestamp::DistantPast,
            duration: INIT_HOLD_DURATION,
            rate: Bitrate::INFINITY,
        }
    }
}

pub(super) trait AsValid<T> {
    fn as_valid(&self) -> Option<T>;
}

impl AsValid<Bitrate> for Option<Bitrate> {
    fn as_valid(&self) -> Option<Bitrate> {
        if let Some(bitrate) = self {
            if bitrate.as_f64().is_finite() {
                return Some(*bitrate);
            }
        }
        None
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            observation_window_size: 15,
            observation_duration_lower_bound: Duration::from_millis(250),
            trendline_integration_enabled: false,
            temporal_weight_factor: 0.9,
            instant_upper_bound_temporal_weight_factor: 0.9,
            instant_upper_bound_loss_offset: 0.05,
            instant_upper_bound_bandwidth_balance: Bitrate::kbps(100),
            high_loss_rate_threshold: 1.0,
            slope_of_bwe_high_loss_function: Bitrate::kbps(1000),
            bandwidth_cap_at_high_loss_rate: Bitrate::kbps(500),
            initial_inherent_loss_estimate: 0.01,
            inherent_loss_upper_bound_offset: 0.05,
            inherent_loss_upper_bound_bandwidth_balance: Bitrate::kbps(100),
            inherent_loss_lower_bound: 1.0e-3,
            newton_iterations: 1,
            newton_step_size: 0.75,
            not_increase_if_inherent_loss_less_than_average_loss: true,
            delayed_increase_window: Duration::from_millis(300),
            bandwidth_rampup_upper_bound_factor: 1.5,
            candidate_factor: [1.02, 1.0, 0.95],
            append_acknowledged_rate_candidate: true,
            append_delay_based_estimate_candidate: true,
            bandwidth_backoff_lower_bound_factor: 1.0,
            rampup_acceleration_maxout_time: Duration::from_secs(60),
            rampup_acceleration_max_factor: Duration::from_secs(60),
            higher_bandwidth_bias_factor: 0.0002,
            higher_log_bandwidth_bias_factor: 0.02,
            threshold_of_high_bandwidth_preference: 0.2,
            bandwidth_preference_smoothing_factor: 0.002,
            use_byte_loss_ratio: true,
            hold_duration_factor: 2.0,
            bandwidth_rampup_hold_threshold: 1.3,
            bandwidth_rampup_upper_bound_factor_in_hold: 1.2,
        }
    }
}

impl PacketResult for &TwccSendRecord {
    fn local_send_time(&self) -> Instant {
        (*self).local_send_time()
    }

    fn size(&self) -> DataSize {
        (*self).size().into()
    }

    fn lost(&self) -> bool {
        (*self).remote_recv_time().is_none()
    }
}
