use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use super::super::macros::log_probe_bitrate_estimate;
use super::ProbeClusterConfig;
use crate::util::{f64_to_i64, f64_to_u64, not_happening, u64_as_usize, usize_to_f64};
use crate::{Bitrate, DataSize, TwccClusterId, TwccSendRecord};

/// Minimum ratio of packets we need to receive for a valid probe (80%).
const MIN_RECEIVED_PROBES_RATIO: f64 = 0.80;

/// Minimum ratio of bytes we need to receive for a valid probe (80%).
const MIN_RECEIVED_BYTES_RATIO: f64 = 0.80;

/// Minimum packet count for a valid probe cluster (WebRTC's kMinClusterSize).
const MIN_CLUSTER_SIZE: usize = 4;

/// The maximum valid duration between first and last probe packet on send/receive side.
/// Matches WebRTC's `kMaxProbeInterval` in `probe_bitrate_estimator.cc`.
const MAX_PROBE_INTERVAL: Duration = Duration::from_secs(1);

/// The maximum |receive rate| / |send rate| ratio for a valid estimate.
/// Matches WebRTC's `kMaxValidRatio`.
const MAX_VALID_RATIO: f64 = 2.0;

/// Minimum |receive rate| / |send rate| ratio to consider the link unsaturated.
/// Matches WebRTC's `kMinRatioForUnsaturatedLink`.
const MIN_RATIO_FOR_UNSATURATED_LINK: f64 = 0.9;

/// Target utilization when we believe we've found the true capacity.
/// Matches WebRTC's `kTargetUtilizationFraction`.
const TARGET_UTILIZATION_FRACTION: f64 = 0.95;

/// Maximum number of active probes before we trigger cleanup.
const MAX_ACTIVE_PROBES: usize = 20;

/// Probes older than this are considered stale and will be removed when hitting the cap.
const STALE_PROBE_THRESHOLD: Duration = Duration::from_secs(5);

/// Analyzes probe cluster results from TWCC feedback.
///
/// This component takes packets tagged with a `TwccClusterId` and calculates the
/// achieved bitrate for each probe cluster.
///
/// **Important:** This follows WebRTC's `ProbeBitrateEstimator` semantics:
/// only packets with a known remote receive timestamp are included in the probe
/// result. Probe packets reported as lost (no remote receive timestamp) are ignored.
#[derive(Debug)]
pub struct ProbeEstimator {
    /// Active probe states (VecDeque for efficient front removal).
    states: VecDeque<ProbeEstimatorState>,

    /// Clusters that were updated in the last call to `update`.
    did_update: VecDeque<TwccClusterId>,
}

#[derive(Debug)]
struct ProbeEstimatorState {
    /// Configuration of the active probe (targets for validation).
    config: ProbeClusterConfig,

    /// When this probe was created.
    created_at: Instant,

    /// When to erase this cluster's state (cluster history expiry).
    finalize_at: Instant,

    /// First (earliest) send time among packets included in this probe.
    first_send_time: Option<Instant>,
    /// Last (latest) send time among packets included in this probe.
    last_send_time: Option<Instant>,
    /// Size of the packet with the last send time (excluded from send-rate calculation).
    size_last_send: DataSize,

    /// First (earliest) receive time among packets included in this probe.
    first_recv_time: Option<Instant>,
    /// Last (latest) receive time among packets included in this probe.
    last_recv_time: Option<Instant>,
    /// Size of the packet with the first receive time (excluded from receive-rate calculation).
    size_first_receive: DataSize,

    /// Total bytes for packets included in this probe (received packets only).
    total_bytes: DataSize,
    /// Number of packets included in this probe (received packets only).
    packet_count: usize,
}

impl ProbeEstimator {
    pub fn new() -> Self {
        Self {
            states: VecDeque::new(),
            did_update: VecDeque::with_capacity(10),
        }
    }

    /// Start analyzing a new probe cluster.
    ///
    /// Resets all accumulated state and begins watching for packets with the
    /// given cluster ID. Returns `true` if the probe was started, `false` if
    /// it was rejected due to too many active probes.
    pub fn probe_start(&mut self, config: ProbeClusterConfig, now: Instant) -> bool {
        // Under normal operation, we expect at most 2-4 active probes:
        // - Initial exponential probing: 2 probes (3×, 6×)
        // - Further probing: 1-2 additional probes
        // - Allocation/recovery probing: 1-2 more
        // Even with rapid probe sequences and 1-second cleanup delay, we shouldn't
        // accumulate more than ~8 probes. If we hit 20, it indicates a bug in probe
        // lifecycle management (missing end_probe() calls or handle_timeout() not running).

        // If we've hit the cap, try to clean up stale probes first.
        if self.states.len() >= MAX_ACTIVE_PROBES {
            let before_cleanup = self.states.len();
            self.states
                .retain(|s| now.saturating_duration_since(s.created_at) < STALE_PROBE_THRESHOLD);

            let removed = before_cleanup - self.states.len();
            if removed > 0 {
                tracing::debug!(
                    removed,
                    remaining = self.states.len(),
                    "Cleaned up stale probes"
                );
            }

            // If still at cap after cleanup, reject the new probe.
            if self.states.len() >= MAX_ACTIVE_PROBES {
                tracing::debug!("Rejecting new probe: too many active probes with none stale");
                return false;
            }
        }

        self.states.push_back(ProbeEstimatorState::new(config, now));
        true
    }

    /// Process TWCC feedback records.
    ///
    /// Only accumulates packets that match the active cluster ID. All other
    /// packets are ignored.
    pub fn update<'t>(
        &mut self,
        records: impl Iterator<Item = &'t TwccSendRecord>,
    ) -> impl Iterator<Item = (ProbeClusterConfig, Bitrate)> + '_ {
        // Keep track of which clusters were updated in this call.
        self.did_update.clear();

        for record in records {
            let Some(cluster) = record.cluster() else {
                continue;
            };

            // Find the state for this cluster.
            let maybe_state = self
                .states
                .iter_mut()
                .find(|s| s.config.cluster() == cluster);

            let Some(state) = maybe_state else {
                continue;
            };

            let did_update = state.update(record);

            if did_update {
                // The correct behavior is that the _last updated_
                // is emitted last, so that the consumer of the returned
                // iterator gets the latest probe result last.
                self.did_update.retain(|c| *c != cluster);
                self.did_update.push_back(cluster);
            }
        }

        self.did_update
            .iter()
            .filter_map(|cluster| self.states.iter().find(|s| s.config.cluster() == *cluster))
            .filter_map(ProbeEstimatorState::calculate_bitrate)
    }

    /// Mark the probe as ended.
    ///
    /// The probe will continue collecting feedback during a cluster history
    /// period after the probe is finished. This period must be shorter than
    /// the time between probe clusters to avoid overlap.
    pub fn end_probe(&mut self, now: Instant, cluster_id: TwccClusterId) {
        let maybe_state = self
            .states
            .iter_mut()
            .find(|s| s.config.cluster() == cluster_id);

        let Some(state) = maybe_state else {
            return;
        };

        state.end_probe(now);
    }

    pub fn poll_timeout(&self) -> Instant {
        self.states
            .iter()
            .map(|s| s.finalize_at)
            .min()
            .unwrap_or(not_happening())
    }

    /// Finalize probes that are ready.
    pub fn handle_timeout(&mut self, now: Instant) {
        self.states.retain(|s| {
            let do_keep = now < s.finalize_at;
            if do_keep {
                return true;
            }

            let result = s.do_calculate_bitrate();
            if let ProbeResult::Estimate(_) = result {
                // Already logged in calculate_bitrate() during update().
            } else {
                // Log the final rejection reason for the probe.
                tracing::trace!(%result, "Probe result");
            }

            false
        });
    }

    /// Clear all active probes.
    ///
    /// This should be called when probing is no longer possible.
    pub fn clear_probes(&mut self) {
        self.states.clear();
    }
}

impl ProbeEstimatorState {
    pub fn new(config: ProbeClusterConfig, now: Instant) -> Self {
        Self {
            config,
            created_at: now,
            finalize_at: not_happening(),
            first_send_time: None,
            last_send_time: None,
            size_last_send: DataSize::ZERO,
            first_recv_time: None,
            last_recv_time: None,
            size_first_receive: DataSize::ZERO,
            total_bytes: DataSize::ZERO,
            packet_count: 0,
        }
    }

    fn update(&mut self, record: &TwccSendRecord) -> bool {
        // Only packets with a known remote receive time participate in probe estimation.
        let Some(recv_time) = record.remote_recv_time() else {
            return false; // lost/unreceived packet -> ignore for probe result
        };

        let packet_size = DataSize::from(record.size());
        let send_time = record.local_send_time();

        // Track min/max send time among included packets.
        let first = self.first_send_time.get_or_insert(send_time);
        *first = (*first).min(send_time);

        let last = self.last_send_time.get_or_insert(send_time);
        if send_time >= *last {
            *last = send_time;
            self.size_last_send = packet_size;
        }

        // Track min/max receive time among included packets.
        let first_recv = self.first_recv_time.get_or_insert(recv_time);
        if recv_time <= *first_recv {
            *first_recv = recv_time;
            self.size_first_receive = packet_size;
        }

        let last_recv = self.last_recv_time.get_or_insert(recv_time);
        *last_recv = (*last_recv).max(recv_time);

        self.total_bytes += packet_size;
        self.packet_count += 1;

        true
    }

    fn calculate_bitrate(&self) -> Option<(ProbeClusterConfig, Bitrate)> {
        let result = self.do_calculate_bitrate();

        let ProbeResult::Estimate(bitrate) = result else {
            return None;
        };

        // Log the estimates continuously during the probe.
        tracing::trace!(%result, "Probe result");
        log_probe_bitrate_estimate!(bitrate.as_f64());

        Some((self.config, bitrate))
    }

    /// Calculate the estimated bitrate for this probe cluster.
    fn do_calculate_bitrate(&self) -> ProbeResult {
        // WebRTC requires at least kMinClusterSize (4) packets received.
        // We may send more, but packet loss can result in fewer received packets.
        if self.packet_count < MIN_CLUSTER_SIZE {
            return ProbeResult::ClusterTooSmall {
                recv: self.packet_count,
                limit: MIN_CLUSTER_SIZE,
            };
        }

        // Also check we received enough of what was sent
        let min_packets = u64_as_usize(f64_to_u64(
            usize_to_f64(self.config.min_packet_count()) * MIN_RECEIVED_PROBES_RATIO,
        ));
        let min_bytes = DataSize::bytes(f64_to_i64(
            usize_to_f64(self.config.target_bytes().as_bytes_usize()) * MIN_RECEIVED_BYTES_RATIO,
        ));

        if self.packet_count < min_packets {
            return ProbeResult::InsufficientPackets {
                recv: self.packet_count,
                limit: min_packets,
            };
        }
        if self.total_bytes < min_bytes {
            return ProbeResult::InsufficientBytes {
                recv: self.total_bytes,
                limit: min_bytes,
            };
        }

        // Get timing bounds
        let Some(first_send) = self.first_send_time else {
            return ProbeResult::MissingTimingInfo;
        };
        let Some(last_send) = self.last_send_time else {
            return ProbeResult::MissingTimingInfo;
        };
        let send_interval = last_send.saturating_duration_since(first_send);

        let Some(first_recv) = self.first_recv_time else {
            return ProbeResult::MissingTimingInfo;
        };
        let Some(last_recv) = self.last_recv_time else {
            return ProbeResult::MissingTimingInfo;
        };
        let recv_interval = last_recv.saturating_duration_since(first_recv);

        // Intervals must be positive and within bounds.
        if send_interval.is_zero() {
            return ProbeResult::SendIntervalInvalid {
                interval: send_interval,
            };
        }
        if send_interval > MAX_PROBE_INTERVAL {
            return ProbeResult::SendIntervalTooLong {
                interval: send_interval,
            };
        }
        if recv_interval.is_zero() || recv_interval > MAX_PROBE_INTERVAL {
            return ProbeResult::RecvIntervalInvalid {
                interval: recv_interval,
            };
        }

        // WebRTC boundary exclusions:
        // - exclude the last sent packet size when computing send rate
        // - exclude the first received packet size when computing receive rate
        let send_size = self.total_bytes.saturating_sub(self.size_last_send);
        let recv_size = self.total_bytes.saturating_sub(self.size_first_receive);
        if send_size <= DataSize::ZERO || recv_size <= DataSize::ZERO {
            return ProbeResult::InvalidDataSize;
        }

        let recv_rate = recv_size / recv_interval;
        let send_rate = send_size / send_interval;

        // WebRTC validation: reject if receive/send ratio is too high.
        let ratio = recv_rate.as_f64() / send_rate.as_f64();
        if ratio > MAX_VALID_RATIO {
            return ProbeResult::InvalidSendReceiveRatio {
                ratio,
                limit: MAX_VALID_RATIO,
            };
        }

        // Match WebRTC semantics:
        // - estimate is the min(send_rate, recv_rate)
        // - if recv_rate is significantly lower than send_rate, assume saturation and
        //   return a conservative fraction of recv_rate.
        let mut estimate = send_rate.min(recv_rate);
        if recv_rate < send_rate * MIN_RATIO_FOR_UNSATURATED_LINK {
            estimate = recv_rate * TARGET_UTILIZATION_FRACTION;
        }

        ProbeResult::Estimate(estimate)
    }

    fn end_probe(&mut self, now: Instant) {
        self.finalize_at = now + Duration::from_secs(1);
    }
}

/// Result of a probe cluster estimation.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ProbeResult {
    /// Successfully estimated bitrate
    Estimate(Bitrate),
    /// Not enough packets in cluster (< 4)
    ClusterTooSmall { recv: usize, limit: usize },
    /// Insufficient packets received (< 80% of sent)
    InsufficientPackets { recv: usize, limit: usize },
    /// Insufficient bytes received (< 80% of sent)
    InsufficientBytes { recv: DataSize, limit: DataSize },
    /// Send interval too long (> 1 second)
    SendIntervalTooLong { interval: Duration },
    /// Send interval invalid (zero)
    SendIntervalInvalid { interval: Duration },
    /// Receive interval invalid (zero or > 1 second)
    RecvIntervalInvalid { interval: Duration },
    /// Invalid receive/send ratio (recv_rate / send_rate too high)
    InvalidSendReceiveRatio { ratio: f64, limit: f64 },
    /// Calculated data size is zero
    InvalidDataSize,
    /// Missing timing information
    MissingTimingInfo,
}

impl fmt::Display for ProbeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeResult::Estimate(bitrate) => write!(f, "estimate={bitrate}"),
            ProbeResult::ClusterTooSmall {
                recv: received,
                limit: required,
            } => {
                write!(f, "cluster too small ({received} < {required})")
            }
            ProbeResult::InsufficientPackets {
                recv: received,
                limit: required,
            } => {
                write!(f, "insufficient packets ({received} < {required})")
            }
            ProbeResult::InsufficientBytes {
                recv: received,
                limit: required,
            } => {
                write!(f, "insufficient bytes ({received} < {required})")
            }
            ProbeResult::SendIntervalTooLong { interval } => {
                write!(f, "send interval too long ({interval:?})")
            }
            ProbeResult::SendIntervalInvalid { interval } => {
                write!(f, "send interval invalid ({interval:?})")
            }
            ProbeResult::RecvIntervalInvalid { interval } => {
                write!(f, "recv interval invalid ({interval:?})")
            }
            ProbeResult::InvalidSendReceiveRatio { ratio, limit } => {
                write!(f, "invalid receive/send ratio ({ratio:.3} > {limit:.3})")
            }
            ProbeResult::InvalidDataSize => write!(f, "invalid data size"),
            ProbeResult::MissingTimingInfo => write!(f, "missing timing info"),
        }
    }
}

#[cfg(test)]
mod test;
