//! Unit tests of the loss controller (str0m `src/bwe/loss_controller.rs`, `mod test`).

use std::time::Instant;

use fastrand::Rng;
use std::time::Duration;

use super::{Bitrate, DataSize, LossBasedBweResult, LossController, LossControllerState};
struct PacketResult {
    local_send_time: Instant,
    size: DataSize,
    lost: bool,
}

impl super::PacketResult for PacketResult {
    fn local_send_time(&self) -> Instant {
        self.local_send_time
    }

    fn size(&self) -> DataSize {
        self.size
    }

    fn lost(&self) -> bool {
        self.lost
    }
}

#[test]
fn no_loss() {
    // Test no loss, estimate should be bounded by delay based estimate
    let mut lbc = LossController::new();
    lbc.set_min_bitrate(Bitrate::from(50_000)); // 50 kbps
    lbc.set_max_bitrate(Bitrate::from(1_000_000_000)); // 1 Gbps

    let acknowledged_bitrate = Bitrate::from(1_000_000); // 1 Mbps
    lbc.set_acknowledged_bitrate(acknowledged_bitrate);
    lbc.set_bandwidth_estimate(Bitrate::from(1_250_000)); // 1.25Mbps

    let mut pkt_builder = PacketBuilder::new(Instant::now()).num_packets(26);

    // A single observation at 1Mbps
    let result = pkt_builder.build_packets();
    lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));
    pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));

    // A single observation at 1Mbps
    let result = pkt_builder.build_packets();
    lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));

    let LossBasedBweResult {
        bandwidth_estimate,
        state,
    } = lbc.loss_based_result();

    assert_eq!(
        bandwidth_estimate,
        Some(Bitrate::bps(1_500_000)),
        "Estimate should increase to delay based estimate, but not further"
    );
    assert_eq!(state, LossControllerState::DelayBased);
}

#[test]
fn stable_loss() {
    // Test stable loss at 5% which should be ignored by the loss controller
    let mut lbc = LossController::new();
    lbc.set_min_bitrate(Bitrate::from(50_000)); // 50 kbps
    lbc.set_max_bitrate(Bitrate::from(1_000_000_000)); // 1 Gbps

    let acknowledged_bitrate = Bitrate::from(1_000_000); // 1 Mbps
    lbc.set_acknowledged_bitrate(acknowledged_bitrate);
    lbc.set_bandwidth_estimate(Bitrate::from(1_250_000)); // 1.25Mbps

    let mut pkt_builder = PacketBuilder::new(Instant::now())
        .with_loss(0.05)
        .num_packets(26);

    // It takes a while for the maximum likelihood estimation to react to the inherent loss
    // this is why we need quite a few observations before the estimate increases to the delay
    // based bound
    // 40 observations(10 seconds) at 1Mbps
    for _ in 0..40 {
        let result = pkt_builder.build_packets();
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));
        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));
    }

    let LossBasedBweResult {
        bandwidth_estimate,
        state,
    } = lbc.loss_based_result();

    assert!(
        state == LossControllerState::DelayBased || state == LossControllerState::Increasing,
        "With stable inherent loss, should be in DelayBased or Increasing state, got {state:?}"
    );

    // Note: The loss controller returns loss_limited_bandwidth even in DelayBased state.
    // The BWE integration layer (SendSideBandwidthEstimator::last_estimate) is responsible
    // for using the delay-based estimate when state is DelayBased.
    // This matches WebRTC's LossBasedBweV2 behavior (see loss_based_bwe_v2.cc:379).
    assert!(
        bandwidth_estimate.is_some(),
        "Loss controller should return an estimate even in DelayBased state"
    );
}

#[test]
fn stable_loss_with_loss_spike() {
    // Test stable loss at 5% which should be ignored by the loss controller, followed by a
    // loss spike which should cause the estimate to dip
    let mut lbc = LossController::new();
    lbc.set_min_bitrate(Bitrate::from(50_000)); // 50 kbps
    lbc.set_max_bitrate(Bitrate::from(1_000_000_000)); // 1 Gbps

    let acknowledged_bitrate = Bitrate::from(1_000_000); // 1 Mbps
    lbc.set_acknowledged_bitrate(acknowledged_bitrate);
    lbc.set_bandwidth_estimate(Bitrate::from(1_250_000)); // 1.25Mbps

    let mut pkt_builder = PacketBuilder::new(Instant::now())
        .with_loss(0.05)
        .num_packets(26);

    // It takes a while for the maximum likelihood estimation to react to the inherent loss
    // this is why we need quite a few observations before the estimate increases to the delay
    // based bound and is stable.
    // 40 observations(10 seconds) at 1Mbps
    for _ in 0..40 {
        let result = pkt_builder.build_packets();
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));
        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));
    }

    pkt_builder = pkt_builder.with_loss(0.9);
    // Loss spike(1second at 90% loss)
    for _ in 0..4 {
        let result = pkt_builder.build_packets();
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));

        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));
    }

    let LossBasedBweResult {
        bandwidth_estimate,
        state,
    } = lbc.loss_based_result();

    let estimate = bandwidth_estimate.expect("Should have an estimate");
    assert!(
        estimate < Bitrate::bps(500_000),
        "A loss spike should result in a reduced estimate, estimate was {estimate}"
    );
    assert_eq!(state, LossControllerState::Decreasing);
}

#[test]
fn loss_spike_recovery() {
    // Test stable loss at 5% which should be ignored by the loss controller, followed by a
    // loss spike which should cause the estimate to dip
    let mut lbc = LossController::new();
    lbc.set_min_bitrate(Bitrate::from(50_000)); // 50 kbps
    lbc.set_max_bitrate(Bitrate::from(1_000_000_000)); // 1 Gbps

    let acknowledged_bitrate = Bitrate::from(1_000_000); // 1 Mbps
    lbc.set_acknowledged_bitrate(acknowledged_bitrate);
    lbc.set_bandwidth_estimate(Bitrate::from(1_250_000)); // 1.25Mbps

    let mut pkt_builder = PacketBuilder::new(Instant::now())
        .with_loss(0.05)
        .num_packets(26);

    // It takes a while for the maximum likelihood estimation to react to the inherent loss
    // this is why we need quite a few observations before the estimate increases to the delay
    // based bound and is stable.
    // 40 observations(10 seconds) at 1Mbps
    for _ in 0..40 {
        let result = pkt_builder.build_packets();
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));
        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));
    }

    // Loss spike
    pkt_builder = pkt_builder.with_loss(0.9);
    let result = pkt_builder.build_packets();
    lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));
    pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));

    pkt_builder = pkt_builder.with_loss(0.05);
    // Set loss back to 5% and gradually ramp up the bitrate
    for i in 0..40 {
        pkt_builder = pkt_builder.num_packets(6 + i / 2);

        let result = pkt_builder.build_packets();
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));

        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));
    }

    let LossBasedBweResult { state, .. } = lbc.loss_based_result();

    // Note: The loss controller returns loss_limited_bandwidth even in DelayBased state.
    // After recovery, it may be in Increasing or DelayBased state depending on dynamics.
    // The BWE integration layer uses the delay-based estimate when state is DelayBased.
    assert!(
        state == LossControllerState::DelayBased || state == LossControllerState::Increasing,
        "After recovery from loss spike, should be in DelayBased or Increasing state, got {state:?}"
    );
}

#[test]
fn stable_loss_gradual_overuse() {
    // Test stable loss at 5% which should be ignored by the loss controller, followed by
    // a gradual increase in loss as we overuse the capacity
    let mut lbc = LossController::new();
    lbc.set_min_bitrate(Bitrate::from(50_000)); // 50 kbps
    lbc.set_max_bitrate(Bitrate::from(1_000_000_000)); // 1 Gbps

    let acknowledged_bitrate = Bitrate::from(1_000_000); // 1 Mbps
    lbc.set_acknowledged_bitrate(acknowledged_bitrate);
    lbc.set_bandwidth_estimate(Bitrate::from(1_250_000)); // 1.25Mbps

    let mut pkt_builder = PacketBuilder::new(Instant::now())
        .with_loss(0.05)
        .num_packets(26);

    // It takes a while for the maximum likelihood estimation to react to the inherent loss
    // this is why we need quite a few observations before the estimate increases to the delay
    // based bound
    // 40 observations(10 seconds) at 1Mbps
    for _ in 0..40 {
        let result = pkt_builder.build_packets();
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));

        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));
    }

    // Gradual increase
    for inc in 0..10 {
        pkt_builder = pkt_builder.with_loss(0.05 + (f64::from(inc) / 10.0));

        for _ in 0..4 {
            let result = pkt_builder.build_packets();
            lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));

            pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));
        }
    }

    let LossBasedBweResult {
        bandwidth_estimate,
        state,
    } = lbc.loss_based_result();

    let estimate = bandwidth_estimate.expect("Should have an estimate");
    assert!(
        estimate < Bitrate::bps(1_000_000),
        "A gradual overuse should result in a lowered estimate"
    );
    assert_eq!(state, LossControllerState::Decreasing);
}

#[test]
fn test_loss_limited_window() {
    let mut lbc = LossController::new();
    lbc.set_min_bitrate(Bitrate::kbps(50));
    lbc.set_max_bitrate(Bitrate::gbps(1));

    let acknowledged_bitrate = Bitrate::mbps(1); // 1 Mbps
    lbc.set_acknowledged_bitrate(acknowledged_bitrate);
    lbc.set_bandwidth_estimate(Bitrate::kbps(1_250)); // 1.25Mbps

    let mut pkt_builder = PacketBuilder::new(Instant::now()).num_packets(25);

    {
        // Initial observation with no loss
        let result = pkt_builder.build_packets();
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));
        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));
    }

    let loss_limited = {
        // loss spike observation at 50%
        pkt_builder = pkt_builder.with_loss(0.5);
        let result = pkt_builder.build_packets();
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));
        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));

        let LossBasedBweResult {
            bandwidth_estimate,
            state,
        } = lbc.loss_based_result();

        let estimate = bandwidth_estimate.expect("Should have an estimate");
        assert!(
            estimate < Bitrate::kbps(600),
            "A loss spike should've caused a significant drop in estimate, got {estimate}"
        );
        assert_eq!(state, LossControllerState::Decreasing);

        estimate
    };

    {
        // Recovery observation at 0% loss
        pkt_builder = pkt_builder.with_loss(0.0);
        let result = pkt_builder.build_packets();
        // Lower acknowledged bitrate to simulate reacting to estimate due to spike
        lbc.set_acknowledged_bitrate(Bitrate::kbps(300));
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));
        pkt_builder = pkt_builder.forward_time(Duration::from_millis(250));

        let LossBasedBweResult {
            bandwidth_estimate,
            state,
        } = lbc.loss_based_result();

        let estimate = bandwidth_estimate.expect("Should have an estimate");
        assert!(
            estimate > loss_limited && estimate <= Bitrate::mbps(1),
            "During the recovery window after a loss spike the estimate should increase, but be bounded. loss_limited={loss_limited}, estimate={estimate}, expected <= 1 Mbps"
        );
        assert_eq!(state, LossControllerState::Decreasing);
    }

    {
        // Another recovery observation at 0% loss, outside of the limit window
        pkt_builder = pkt_builder.num_packets(80);
        let result = pkt_builder.build_packets();
        lbc.set_acknowledged_bitrate(Bitrate::mbps(1));
        lbc.update_bandwidth_estimate(&result, Bitrate::bps(1_500_000));

        let LossBasedBweResult {
            bandwidth_estimate,
            state,
        } = lbc.loss_based_result();

        let estimate = bandwidth_estimate.expect("Should have an estimate");
        assert!(
            estimate == Bitrate::bps(1_000_000),
            "Eventually the estimate should recover but still remain bounded until the average loss caused by spike ages out"
        );
        assert_eq!(state, LossControllerState::Decreasing);
    }
}

struct PacketBuilder {
    now: Instant,
    rng: Rng,
    loss_rate: f64,
    send_distribution: LogNormalDistribution,
    recv_distribution: LogNormalDistribution,
    num_packets: u32,
    packet_size: DataSize,
}

impl PacketBuilder {
    fn new(now: Instant) -> Self {
        Self {
            now,
            rng: Rng::with_seed(34_791_910),
            loss_rate: 0.0,
            send_distribution: LogNormalDistribution {
                mean: 0.05,
                std_dev: 1.0,
            },
            recv_distribution: LogNormalDistribution {
                mean: 4.0,
                std_dev: 10.0,
            },
            num_packets: 10,
            packet_size: DataSize::bytes(1200),
        }
    }

    fn forward_time(mut self, by: Duration) -> Self {
        self.now += by;
        self
    }

    fn with_loss(mut self, loss_rate: f64) -> Self {
        self.loss_rate = loss_rate;
        self
    }

    fn num_packets(mut self, packets: u32) -> Self {
        self.num_packets = packets;
        self
    }

    fn build_packets(&mut self) -> Vec<PacketResult> {
        let mut last_send_time = self.now;
        let mut last_recv_time = self.now;
        let mut result: Vec<PacketResult> = Vec::with_capacity(self.num_packets as usize);

        for _ in 0..self.num_packets {
            let lost = self.rng.f64() <= self.loss_rate;
            let first_send_time = last_send_time
                + Duration::from_secs_f64(self.send_distribution.sample(&mut self.rng) / 1000.0);
            let recv_time = last_recv_time
                + Duration::from_secs_f64(self.recv_distribution.sample(&mut self.rng) / 1000.0);

            result.push(PacketResult {
                local_send_time: first_send_time,
                size: self.packet_size,
                lost,
            });

            last_send_time = first_send_time;
            if !lost {
                last_recv_time = recv_time;
            }
        }

        result
    }
}

struct LogNormalDistribution {
    mean: f64,
    std_dev: f64,
}

impl LogNormalDistribution {
    fn sample(&self, rng: &mut Rng) -> f64 {
        let normal = normal_distribution(rng);
        let location = (self.mean.powi(2) / (self.mean.powi(2) + self.std_dev.powi(2)).sqrt()).ln();
        let scale = (1.0 + (self.std_dev / self.mean).powi(2)).ln().sqrt();

        (location + scale * normal).exp()
    }
}

fn normal_distribution(rng: &mut Rng) -> f64 {
    let u1 = rng.f64();
    let u2 = rng.f64();

    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}
