use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::Pacer;
use super::PaddingRequest;
use super::QueueState;
use crate::bwe::ProbeClusterConfig;
use crate::bwe::ProbeClusterState;
use crate::bwe::{log_pacer_media_debt, log_pacer_padding_debt};
use crate::pacer::PacerReason;
use crate::util::Soonest;
use crate::Reason;
use crate::{Bitrate, DataSize, QueueId, TwccClusterId};

const MAX_BITRATE: Bitrate = Bitrate::gbps(10);
const MAX_DEBT_IN_TIME: Duration = Duration::from_millis(500);
const PADDING_BURST_INTERVAL: Duration = Duration::from_millis(5);
const PACING: Duration = Duration::from_millis(40);
/// Max size of a burst of paced media packets, to not risk overfilling socket buffers at
/// high bitrates. Same as libwebrtc's `PacingController::kMaxBurstSize`.
const MAX_BURST_SIZE: DataSize = DataSize::bytes(63_000);

/// A leaky bucket pacer that can overshoot the target bitrate when required.
pub struct LeakyBucketPacer {
    /// Pacing bitrate.
    pacing_bitrate: Bitrate,
    /// Adjusted pacing bitrate for when we need to drain queues.
    adjusted_bitrate: Bitrate,
    /// The bitrate at which to send padding packets when the pacing rate isn't being achieved.
    padding_bitrate: Bitrate,
    /// The last time we refreshed media debt and potentially adjusted the bitrate.
    last_handle_time: Option<Instant>,
    /// The last time we indicated that a packet should be sent.
    last_emitted: Option<Instant>,
    /// The next time we should send a queued packet.
    next_poll_time: Option<(Instant, PacerReason)>,
    /// The current media debt.
    media_debt: DataSize,
    /// The current padding debt.
    padding_debt: DataSize,
    /// The longest the average packet can spend in the queue before we force it to be drained.
    queue_limit: Duration,
    /// The queue states given by last handle_timeout.
    queue_states: Vec<QueueState>,
    /// The next return value for `poll_queue``
    next_poll_queue: Option<QueueId>,
    /// Queue of probe clusters waiting to be executed
    probe_queue: VecDeque<ProbeClusterState>,
    /// Last completed probe cluster (to be consumed by check_probe_complete)
    completed_probe: Option<TwccClusterId>,
    /// Gates poll_queue() until handle_timeout() is called after packet emission.
    needs_timeout_before_next_poll: bool,
}

impl Pacer for LeakyBucketPacer {
    fn set_pacing_rate(&mut self, pacing_bitrate: Bitrate) {
        self.pacing_bitrate = pacing_bitrate;

        // bitrate will be updated on next handle_timeout().
    }

    fn set_padding_rate(&mut self, padding_bitrate: Bitrate) {
        self.padding_bitrate = padding_bitrate;

        // bitrate will be updated on next handle_timeout().
    }

    fn poll_timeout(&self) -> (Option<Instant>, Reason) {
        let next_handle_time = self.last_handle_time.map(|lh| lh + PACING);

        let poll_at = self
            .next_poll_time
            .map_or((None, Reason::NotHappening), |(t, r)| {
                (Some(t), Reason::Pacer(r))
            });

        (next_handle_time, Reason::Pacer(PacerReason::Handle)).soonest(poll_at)
    }

    fn handle_timeout(
        &mut self,
        now: Instant,
        iter: impl Iterator<Item = QueueState>,
    ) -> Option<PaddingRequest> {
        // Clear the gate when time advances
        self.needs_timeout_before_next_poll = false;

        // Clear the poll time - it will be recalculated below if needed.
        // This is important because if we return early (e.g., next_poll_queue is already set),
        // we don't want the old Immediate timeout to keep firing.
        self.next_poll_time = None;

        // This is called periodically and whenever packet is queued.
        self.queue_states.clear();
        self.queue_states.extend(iter);

        let elapsed = self.update_handle_time_and_get_elapsed(now);

        self.clear_debt(elapsed);
        self.maybe_update_adjusted_bitrate(now);

        if let Some(request) = self.maybe_create_padding_request(now) {
            self.next_poll_queue = Some(request.queue_id);
            return Some(request);
        }

        if self.next_poll_queue.is_some() {
            return None;
        }

        let (next_poll_time_and_reason, queue) = self.next_poll(now)?;

        if now < next_poll_time_and_reason.0 {
            // We don't set this because between now and the neaxt poll, queue state can change such
            // that we should poll a different queue i.e. media could be queued.
            self.next_poll_queue = None;
        } else {
            self.next_poll_queue = queue.map(|q| q.queue_id);
        }

        self.next_poll_time = Some(next_poll_time_and_reason);

        None
    }

    fn poll_queue(&mut self) -> Option<(QueueId, Option<TwccClusterId>)> {
        // GATE: Block if we need timeout first
        if self.needs_timeout_before_next_poll {
            return None;
        }

        let next = self.next_poll_queue.take()?;

        // Mark that we need timeout before next poll
        self.needs_timeout_before_next_poll = true;
        self.request_immediate_timeout();

        // Capture the cluster ID at poll time, before register_send() might clear it
        let cluster_id = self.active_cluster();

        Some((next, cluster_id))
    }

    fn register_send(&mut self, now: Instant, packet_size: DataSize, _from: QueueId) {
        self.last_emitted = Some(now);

        self.media_debt += packet_size;
        self.media_debt = self
            .media_debt
            .min(self.adjusted_bitrate * MAX_DEBT_IN_TIME);
        log_pacer_media_debt!(self.media_debt.as_bytes_usize());
        self.add_padding_debt(packet_size);

        // Update active probe state to track this packet
        // This ensures probe timing advances correctly even when sending media packets
        if let Some(probe) = self.probe_queue.front_mut() {
            probe.record_packet(now, packet_size);
        }

        // Check if probe is complete and store it for later retrieval
        if let Some(cluster_id) = self.check_probe_complete_internal(now) {
            self.completed_probe = Some(cluster_id);
        }
    }
}

impl LeakyBucketPacer {
    /// A pacer at `initial_pacing_bitrate`, with a 2 s queue time limit.
    pub fn new(initial_pacing_bitrate: Bitrate) -> Self {
        const DEFAULT_QUEUE_LIMIT: Duration = Duration::from_secs(2);

        Self {
            pacing_bitrate: initial_pacing_bitrate,
            adjusted_bitrate: Bitrate::ZERO,
            padding_bitrate: Bitrate::ZERO,
            last_handle_time: None,
            last_emitted: None,
            next_poll_time: None,
            media_debt: DataSize::ZERO,
            padding_debt: DataSize::ZERO,
            queue_limit: DEFAULT_QUEUE_LIMIT,
            queue_states: vec![],
            next_poll_queue: None,
            probe_queue: VecDeque::new(),
            completed_probe: None,
            needs_timeout_before_next_poll: true,
        }
    }

    /// Set the longest the average packet may spend in the queues before the pacer
    /// raises its rate to drain them (libwebrtc `SetQueueTimeLimit`). The default
    /// is 2 s.
    ///
    /// str0m fixes this at its default and has no setter; this one writes the same
    /// field, read on the next `handle_timeout`.
    pub fn set_queue_limit(&mut self, queue_limit: Duration) {
        self.queue_limit = queue_limit;
    }

    /// Start executing a probe cluster.
    ///
    /// The pacer will pace at the probe's target bitrate and track packets sent.
    /// Probes are queued and executed sequentially.
    pub fn start_probe(&mut self, config: ProbeClusterConfig) {
        tracing::trace!(?config, "Probe start");
        self.probe_queue.push_back(ProbeClusterState::new(config));
    }

    /// Get the cluster ID of the active probe, if any.
    pub fn active_cluster(&self) -> Option<TwccClusterId> {
        self.probe_queue.front().map(|p| p.config().cluster())
    }

    /// Check if the active probe is complete and should be finished.
    pub fn check_probe_complete(&mut self, now: Instant) -> Option<TwccClusterId> {
        // Check if we have a completed probe from a previous call
        if let Some(cluster_id) = self.completed_probe.take() {
            return Some(cluster_id);
        }

        // Otherwise check if the active probe just completed
        self.check_probe_complete_internal(now)
    }

    /// Internal method to check if probe is complete (doesn't consume completed_probe)
    fn check_probe_complete_internal(&mut self, now: Instant) -> Option<TwccClusterId> {
        let probe = self.probe_queue.front()?;

        if probe.is_complete(now) {
            let cluster_id = probe.config().cluster();
            self.probe_queue.pop_front();
            return Some(cluster_id);
        }

        None
    }

    fn update_handle_time_and_get_elapsed(&mut self, now: Instant) -> Duration {
        // Due the calling code this also happens when a packet is queued in any upstream queue.
        let Some(previous_handle_time) = self.last_handle_time else {
            self.last_handle_time = Some(now);
            return Duration::ZERO;
        };

        let elapsed = now - previous_handle_time;
        self.last_handle_time = Some(now);

        elapsed
    }

    fn clear_debt(&mut self, elapsed: Duration) {
        self.media_debt = self
            .media_debt
            .saturating_sub(self.adjusted_bitrate * elapsed);
        self.padding_debt = self
            .padding_debt
            .saturating_sub(self.padding_bitrate * elapsed);
        log_pacer_media_debt!(self.media_debt.as_bytes_usize());
        log_pacer_padding_debt!(self.padding_debt.as_bytes_usize());
    }

    fn next_poll(&self, now: Instant) -> Option<((Instant, PacerReason), Option<&QueueState>)> {
        // If we have never sent before, do so immediately on an arbitrary non-empty queue.
        if self.last_emitted.is_none() {
            let mut queues = self
                .queue_states
                .iter()
                .filter(|q| q.snapshot.packet_count > 0);

            if let Some(queue) = queues.next() {
                return Some(((now, PacerReason::FirstEver), Some(queue)));
            }
        }

        let unpaced = self
            .queue_states
            .iter()
            .filter(|qs| qs.unpaced)
            .filter_map(|qs| qs.snapshot.first_unsent.map(|t| (t, qs)))
            .min_by_key(|(t, _)| *t);

        // Unpaced packets (such as audio by default) are sent immediately.
        if let Some((queued_at, qs)) = unpaced {
            return Some(((queued_at, PacerReason::Unpaced), Some(qs)));
        }

        let non_empty_queue = {
            let non_empty_queues = self
                .queue_states
                .iter()
                .filter(|q| q.snapshot.packet_count > 0);

            // Send on the non-empty queue with the lowest priority that, was least recently
            // sent on.
            non_empty_queues.min_by_key(|q| (q.snapshot.priority, q.snapshot.last_emitted))
        };

        if let Some(queue) = non_empty_queue {
            if self.adjusted_bitrate > Bitrate::ZERO {
                // Check if we're actively probing and should use probe-specific timing
                let poll_at = if let Some(probe) = self.probe_queue.front() {
                    // During probe: use absolute time directly from probe state
                    (probe.next_probe_time(), PacerReason::Probe1)
                } else {
                    // Normal pacing: use relative offset based on debt
                    let drain_debt_time = self.media_debt / self.adjusted_bitrate;
                    // Limit the burst to MAX_BURST_SIZE so high bitrates don't overfill socket
                    // buffers. Below ~12.6 Mbps this is PACING.
                    let burst_interval = PACING.min(MAX_BURST_SIZE / self.adjusted_bitrate);
                    let next_send_offset = if drain_debt_time >= burst_interval {
                        // If we have incurred too much debt we need to wait to let it clear out before sending
                        // again.
                        drain_debt_time
                    } else {
                        Duration::ZERO
                    };

                    let time = self.last_handle_time.map_or(now, |h| h + next_send_offset);

                    (time, PacerReason::Paced)
                };

                return Some((poll_at, Some(queue)));
            }
        }

        let any_queue_for_padding = self.queue_states.iter().any(|q| q.use_for_padding);
        if !any_queue_for_padding {
            return None;
        }

        // If we're actively probing, use probe timing for padding
        if let Some(probe) = self.probe_queue.front() {
            let next_probe_time = probe.next_probe_time();
            // We explicitly don't return a queue to poll here. We need another call to
            // handle_timeout to request the padding before we can poll the selected queue.
            return Some(((next_probe_time, PacerReason::Probe2), None));
        }

        if self.padding_bitrate == Bitrate::ZERO {
            return None;
        }

        // If all queues are empty and we have a padding rate, wait until we have drained
        // both the media debt and padding debt to send some padding.
        let mut drain_debt_time =
            (self.media_debt / self.adjusted_bitrate).max(self.padding_debt / self.padding_bitrate);
        if drain_debt_time.is_zero() {
            // Give the main loop some time to do something else e.g. queue media.
            drain_debt_time = Duration::from_micros(1);
        }

        let padding_at = self.last_handle_time.map_or(now, |h| h + drain_debt_time);

        // We explicitly don't return a queue to poll here. We need another call to
        // handle_timeout to request the padding before we can poll the selected queue.
        Some(((padding_at, PacerReason::Padding), None))
    }

    fn maybe_update_adjusted_bitrate(&mut self, now: Instant) {
        // Use probe's target bitrate if actively probing, otherwise use pacing bitrate
        self.adjusted_bitrate = if let Some(probe) = self.probe_queue.front() {
            probe.config().target_bitrate()
        } else {
            self.pacing_bitrate
        };

        let (queue_time, queued_packets, queue_size) =
            self.queue_states
                .iter()
                .fold((Duration::ZERO, 0, DataSize::ZERO), |acc, q| {
                    (
                        acc.0 + q.snapshot.total_queue_time(now),
                        acc.1 + q.snapshot.packet_count,
                        acc.2 + DataSize::from(q.snapshot.byte_size),
                    )
                });
        if queued_packets == 0 {
            return;
        }

        let avg_queue_time = queue_time / queued_packets;

        // The average time we want the packet in the queue to at most to wait to drain.
        let target_queue_wait =
            Duration::from_millis(1).max(self.queue_limit.saturating_sub(avg_queue_time));
        // Min data rate to drain what's currently in the queue.
        let min_rate = queue_size / target_queue_wait;
        if min_rate > self.adjusted_bitrate {
            // Min rate exceeds our pacing rate, increase the rate to force drain the queue.
            self.adjusted_bitrate = min_rate.clamp(Bitrate::ZERO, MAX_BITRATE);
        }
    }

    fn add_padding_debt(&mut self, size: DataSize) {
        self.padding_debt += size;
        self.padding_debt = self
            .padding_debt
            .min(self.padding_bitrate * MAX_DEBT_IN_TIME);
        log_pacer_padding_debt!(self.padding_debt.as_bytes_usize());
    }

    /// Optimistically attempt to create a padding request.
    ///
    /// Returns `Some(PaddingRequest)` if padding is enabled and the current queue state
    /// allows padding, otherwise returns `None`.
    fn maybe_create_padding_request(&mut self, now: Instant) -> Option<PaddingRequest> {
        // Queues must be empty.
        let all_queues_empty = self
            .queue_states
            .iter()
            .all(|q| q.snapshot.packet_count == 0);
        if !all_queues_empty {
            return None;
        }

        // We must have a queue that supports padding.
        let maybe_queue = self
            .queue_states
            .iter()
            .filter(|q| q.use_for_padding)
            .max_by_key(|q| q.snapshot.last_emitted);

        if maybe_queue.is_none() {
            // No padding queue, no probes.
            self.probe_queue.clear();
        }

        let queue = maybe_queue?;

        // Check for PROBE padding FIRST (bypasses debt checks)
        // Active probes need padding to hit their target bitrate when there's insufficient media.
        if let Some(probe) = self.probe_queue.front_mut() {
            // Delegate probe timing and padding calculation to ProbeClusterState
            if !probe.should_send_now(now) {
                // Not time yet - wait until next_probe_time
                return None;
            }

            // Get recommended padding amount from ProbeClusterState
            // This handles the calculation of how much padding is needed based on probe timing
            let padding_size = probe.next_packet(now);
            let Some(padding_size) = padding_size else {
                // Probe says no padding needed (already sent enough for this interval)
                return None;
            };

            return Some(PaddingRequest {
                queue_id: queue.queue_id,
                padding: padding_size.as_bytes_usize(),
            });
        }

        // Normal padding: requires zero debt
        if self.media_debt != DataSize::ZERO || self.padding_debt != DataSize::ZERO {
            return None;
        }

        if self.padding_bitrate == Bitrate::ZERO {
            return None;
        }

        // We can generate padding
        let padding = (self.padding_bitrate * PADDING_BURST_INTERVAL).as_bytes_usize();

        Some(PaddingRequest {
            queue_id: queue.queue_id,
            padding,
        })
    }

    fn request_immediate_timeout(&mut self) {
        // Request timeout at the next microsecond to ensure time advances between packets.
        // We can't use already_happened() because that would cause the test harness to
        // set a very old timestamp, and while lib.rs prevents last_now from going backwards,
        // it doesn't force it to advance, so all packets would get the same timestamp.
        const MINIMAL_DELTA: Duration = Duration::from_micros(1);

        let Some(time) = self.last_handle_time.map(|t| t + MINIMAL_DELTA) else {
            self.next_poll_time = None;
            return;
        };

        self.next_poll_time = Some((time, PacerReason::Immediate));
    }
}

#[cfg(test)]
mod test;
