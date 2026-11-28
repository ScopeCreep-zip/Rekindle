//! Probe cluster data structures for bandwidth estimation.
//!
//! Probe clusters are short bursts of packets sent at specific bitrates to test
//! network capacity.

use std::cmp;
use std::time::{Duration, Instant};

use crate::util::{already_happened, not_happening};
use crate::{Bitrate, DataSize, TwccClusterId};

const MAX_PADDING_PACKET_SIZE: DataSize = DataSize::bytes(240);

/// Configuration for a probe cluster (the plan).
///
/// This represents the immutable blueprint for a bandwidth probe: what bitrate
/// to test, for how long, and with what constraints.
#[derive(Debug, Clone, Copy)]
pub struct ProbeClusterConfig {
    /// Unique identifier for this probe cluster
    cluster: TwccClusterId,

    /// Target bitrate to probe at (e.g., 3 Mbps)
    target_bitrate: Bitrate,

    /// How long to sustain the target bitrate (e.g., 15ms)
    target_duration: Duration,

    /// Minimum number of packets to send (e.g., 5)
    /// This ensures statistical validity even for short bursts.
    min_packet_count: usize,

    /// Delta time between sent bursts of packets during probe.
    ///
    /// Mirrors WebRTC's `ProbeClusterConfig.min_probe_delta`. The pacer/probe sender must not
    /// schedule probe packets more frequently than this.
    min_probe_delta: Duration,

    /// The kind of probe this is.
    kind: ProbeKind,
}

/// Kind of probe
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProbeKind {
    /// The first probes at startup.
    Initial,
    /// Follow-up probes while each one keeps succeeding.
    Exponential,
    /// A probe when the desired bitrate rises while application-limited.
    IncreaseAlr,
    /// The periodic probe while application-limited.
    PeriodicAlr,
    /// A probe after a large drop in the estimate.
    LargeDrop,
    /// A probe when the estimate has not moved for a long time.
    Stagnant,
}

impl ProbeKind {
    pub(crate) fn is_alr(&self) -> bool {
        matches!(self, ProbeKind::PeriodicAlr | ProbeKind::IncreaseAlr)
    }
}

impl ProbeClusterConfig {
    /// Create a new probe cluster configuration with standard defaults:
    /// - 15ms duration (enough to get meaningful feedback without excessive delay)
    /// - 5 minimum packets (statistical significance for BWE analysis)
    pub fn new(cluster: TwccClusterId, target_bitrate: Bitrate, kind: ProbeKind) -> Self {
        Self {
            cluster,
            target_bitrate,
            // WebRTC defaults
            target_duration: Duration::from_millis(15),
            min_packet_count: 5,
            // WebRTC default for general probing (not initial/network-state): 2ms.
            min_probe_delta: Duration::from_millis(2),
            kind,
        }
    }

    /// Set a custom target duration for this probe.
    /// WebRTC uses 100ms for initial probes to allow time for media to start.
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.target_duration = duration;
        self
    }

    /// Set a custom minimum packet count for this probe.
    pub fn with_min_packet_count(mut self, min_packet_count: usize) -> Self {
        self.min_packet_count = min_packet_count;
        self
    }

    /// Set a custom minimum probe delta (spacing constraint) for this probe.
    pub fn with_min_probe_delta(mut self, min_probe_delta: Duration) -> Self {
        self.min_probe_delta = min_probe_delta;
        self
    }

    /// Check if this probe was created during ALR.
    pub fn is_alr_probe(&self) -> bool {
        self.kind.is_alr()
    }

    /// Get the probe cluster ID.
    pub fn cluster(&self) -> TwccClusterId {
        self.cluster
    }

    /// Get the target bitrate.
    pub fn target_bitrate(&self) -> Bitrate {
        self.target_bitrate
    }

    /// Get the minimum packet count required for a valid probe.
    pub fn min_packet_count(&self) -> usize {
        self.min_packet_count
    }

    /// Get the minimum probe delta (spacing constraint).
    pub fn min_probe_delta(&self) -> Duration {
        self.min_probe_delta
    }

    /// Calculate the target bytes for this probe.
    /// This is how much data we expect to send at target_bitrate for target_duration.
    pub fn target_bytes(&self) -> DataSize {
        self.target_bitrate * self.target_duration
    }
}

/// Runtime state of an active probe cluster (the execution).
///
/// This tracks what's actually happening as we send probe packets: how much
/// we've sent, how many packets, and when we started.
#[derive(Debug)]
pub struct ProbeClusterState {
    /// The immutable plan
    config: ProbeClusterConfig,

    /// Total bytes sent so far in this probe
    bytes_sent: DataSize,

    /// Total packets sent so far in this probe
    packets_sent: usize,

    /// When the first packet was sent
    /// None = probe hasn't started yet (still queued)
    started_at: Option<Instant>,

    /// When the last packet was sent
    /// Used to calculate actual send duration (first to last packet)
    last_packet_at: Option<Instant>,

    /// When we last created a padding request
    /// Used to prevent creating multiple padding packets for the same instant
    last_padding_at: Instant,
}

impl ProbeClusterState {
    /// Create a new probe cluster state from a config.
    pub fn new(config: ProbeClusterConfig) -> Self {
        Self {
            config,
            bytes_sent: DataSize::ZERO,
            packets_sent: 0,
            started_at: None,
            last_packet_at: None,
            last_padding_at: already_happened(),
        }
    }

    /// Get the probe configuration.
    pub fn config(&self) -> &ProbeClusterConfig {
        &self.config
    }

    /// Calculates the next probe time based on total bytes sent and target bitrate.
    /// This naturally handles variable packet sizes (media packets can be much larger
    /// than padding packets). Next packet can be sent when:
    /// `now >= start_time + (bytes_sent / target_bitrate)`
    pub fn next_probe_time(&self) -> Instant {
        let Some(started_at) = self.started_at else {
            return already_happened();
        };

        if self.config.target_bitrate == Bitrate::ZERO {
            return not_happening();
        }

        // Packet-level pacing: schedule based on bytes sent at the target bitrate.
        // next_time = start_time + (bytes_sent / target_bitrate)
        let send_duration = self.bytes_sent / self.config.target_bitrate;
        let probe_time = started_at + send_duration;

        let min_burst_time = self.last_padding_at + self.config.min_probe_delta();
        probe_time.max(min_burst_time)
    }

    /// Check if the probe cluster is complete.
    ///
    /// A probe is complete when BOTH conditions are met (matching WebRTC):
    /// 1. Sent bytes >= target_bytes (target_bitrate * target_duration)
    /// 2. Sent packets >= min_packet_count
    ///
    /// Note: WebRTC does NOT check duration for completion. Duration is only used
    /// to calculate the target bytes threshold. This ensures probes complete even
    /// when all packets are sent at the same instant (e.g., in tests or when the
    /// pacer generates bursts faster than wall-clock time advances).
    pub fn is_complete(&self, _now: Instant) -> bool {
        if self.started_at.is_none() {
            return false; // Not started yet
        }

        // Match WebRTC's BitrateProber::ProbeSent() logic:
        // if (sent_bytes >= probe_cluster_min_bytes && sent_probes >= probe_cluster_min_probes)
        let bytes_met = self.bytes_sent >= self.config.target_bytes();
        let packets_met = self.packets_sent >= self.config.min_packet_count;

        bytes_met && packets_met
    }

    /// Check if it's time to send the next probe packet.
    ///
    /// Returns `true` if `now >= next_probe_time()`, meaning we should send a packet now.
    pub fn should_send_now(&self, now: Instant) -> bool {
        let next_time = self.next_probe_time();
        let min_burst_time = self.last_padding_at + self.config.min_probe_delta();
        now >= next_time && now >= min_burst_time
    }

    /// Calculate how much padding should be generated for the next probe packet.
    ///
    /// WebRTC probing uses **bursts** of packets, with a minimum delta between bursts
    /// (`min_probe_delta`). In str0m, a `PaddingRequest` is expressed as a number of bytes
    /// to queue (which may result in multiple RTP padding packets), so we request a burst
    /// sized approximately as:
    ///
    /// `burst_bytes = target_bitrate * min_probe_delta`
    ///
    /// This avoids unintentionally capping probe throughput at `MAX_PADDING_PACKET_SIZE /
    /// min_probe_delta` (e.g. 240 bytes / 2ms = 960 kbit/s), which would be far below the
    /// intended probe bitrate.
    ///
    /// Returns `None` if it's not time to send yet, or if we've already created
    /// a padding packet for this instant.
    pub fn next_packet(&mut self, now: Instant) -> Option<DataSize> {
        if self.started_at.is_none() {
            self.started_at = Some(now);
        }

        // Enforce min burst spacing (`min_probe_delta`) between successive padding requests.
        // (The very first burst is not gated because `last_padding_at` starts at already_happened()).
        if now < self.last_padding_at + self.config.min_probe_delta() {
            return None;
        }

        if now < self.next_probe_time() {
            return None;
        }

        // Check if we've already created a padding packet for this exact instant
        // This prevents creating multiple padding packets when handle_timeout() is
        // called multiple times with the same `now` value
        if self.last_padding_at >= now {
            return None;
        }

        // Mark that we've created padding for this instant
        self.last_padding_at = now;

        // It's time to send a new probe burst.
        // Calculate recommended probe size as target_bitrate * min_probe_delta.
        let min_delta = self.config.min_probe_delta();
        let recommended_probe_size = self.config.target_bitrate * min_delta;

        // Ensure we request at least one padding packet worth of bytes even if min_delta is 0.
        let recommended_probe_size = DataSize::bytes(cmp::max(
            recommended_probe_size.as_bytes_i64(),
            MAX_PADDING_PACKET_SIZE.as_bytes_i64(),
        ));

        // Calculate remaining bytes needed to complete the probe cluster.
        let bytes_remaining = self.config.target_bytes().saturating_sub(self.bytes_sent);

        // Like BitrateProber::ProbeSent, completion requires both the byte and
        // packet thresholds. At low rates the byte target can fit in one packet;
        // keep sending bounded padding until the minimum packet count is met.
        let request_bytes = if bytes_remaining == DataSize::ZERO
            && self.packets_sent < self.config.min_packet_count
        {
            MAX_PADDING_PACKET_SIZE
        } else {
            cmp::min(bytes_remaining, recommended_probe_size)
        };

        if request_bytes == DataSize::ZERO {
            None
        } else {
            Some(request_bytes)
        }
    }

    /// Record a packet that was sent as part of this probe (media or padding).
    ///
    /// This updates the probe's tracking state so that `next_probe_time()` correctly
    /// calculates when the next packet should be sent based on the target bitrate.
    ///
    /// Should be called by the pacer whenever ANY packet is sent during an active probe,
    /// not just padding packets generated by `next_packet()`.
    pub fn record_packet(&mut self, now: Instant, size: DataSize) {
        // If a probe is active and we observe a media packet before any probe-generated padding,
        // treat that as the start of the probe. This keeps timing semantics consistent and ensures
        // `next_probe_time()` enforces `min_probe_delta` from the first packet onwards.
        if self.started_at.is_none() {
            self.started_at = Some(now);
        }

        self.bytes_sent += size;
        self.packets_sent += 1;
        self.last_packet_at = Some(now);
    }
}

#[cfg(test)]
#[path = "cluster_test.rs"]
mod test;
