use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use super::super::macros::{log_trendline_estimate, log_trendline_modified_trend};

use super::super::time::{BweTimestamp, TimeDelta};
use super::super::BandwidthUsage;
use super::arrival_group::InterGroupDelayDelta;
use crate::util::{u128_to_f64, usize_to_f64};

const SMOOTHING_COEF: f64 = 0.9;
const OVER_USE_THRESHOLD_DEFAULT_MS: f64 = 12.5;
const OVER_USE_TIME_THRESHOLD: Duration = Duration::from_millis(10);
const MAX_ADOPT_OFFSET_MS: f64 = 15.0;
const THRESHOLD_GAIN: f64 = 4.0;

const K_UP: f64 = 0.0087;
const K_DOWN: f64 = 0.039;

const DELAY_COUNT_RANGE: RangeInclusive<usize> = 60..=1000;

pub struct TrendlineEstimator {
    /// The window size in packets
    window_size: usize,

    /// The first instant we saw, used as zero point.
    zero_time: Option<BweTimestamp>,

    /// The history of observed delay variations.
    history: VecDeque<Timing>,

    /// The total number of observed delay variations.
    num_delay_variations: usize,

    /// Accumulated delay.
    accumulated_delay: f64,

    /// Last smoothed delay.
    smoothed_delay: f64,

    /// The adaptive delay threshold.
    delay_threshold: f64,

    /// Previous trend
    previous_trend: f64,

    /// If we are overusing, this contains data about the overuse.
    overuse: Option<Overuse>,

    /// The last time we updated the adaptive threshold.
    last_threshold_update: Option<Instant>,

    /// Our current hypothesis about the bandwidth usage.
    hypothesis: BandwidthUsage,
}

impl TrendlineEstimator {
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            zero_time: None,
            history: VecDeque::default(),
            num_delay_variations: 0,
            accumulated_delay: 0.0,
            smoothed_delay: 0.0,
            delay_threshold: OVER_USE_THRESHOLD_DEFAULT_MS,
            previous_trend: 0.0,
            overuse: None,
            last_threshold_update: None,
            hypothesis: BandwidthUsage::Normal,
        }
    }

    pub fn add_delay_observation(&mut self, delay_variation: InterGroupDelayDelta, now: Instant) {
        if self.history.is_empty() {
            self.do_add_to_history(delay_variation);
            return;
        }

        self.do_add_to_history(delay_variation);
        while self.history.len() > self.window_size {
            let _ = self.history.pop_front();
        }

        if self.history.len() == self.window_size {
            assert!(
                self.history
                    .iter()
                    .zip(self.history.iter().skip(1))
                    .fold(true, |acc, (a, b)| {
                        acc && a.remote_recv_time_ms <= b.remote_recv_time_ms
                    }),
                "Out of order history {:?}",
                self.history
            );
            self.update_trendline(delay_variation, now);
        }
    }

    pub fn hypothesis(&self) -> BandwidthUsage {
        self.hypothesis
    }

    fn do_add_to_history(&mut self, variation: InterGroupDelayDelta) {
        let last_remote_recv_time = BweTimestamp::from(variation.last_remote_recv_time);

        let zero_time = *self.zero_time.get_or_insert(last_remote_recv_time);

        let delay_delta = variation.arrival_delta.as_secs_f64() * 1000.0
            - variation.send_delta.as_secs_f64() * 1000.0;

        self.num_delay_variations += 1;
        self.num_delay_variations = self.num_delay_variations.min(*DELAY_COUNT_RANGE.end());
        self.accumulated_delay += delay_delta;
        self.smoothed_delay =
            self.smoothed_delay * SMOOTHING_COEF + (1.0 - SMOOTHING_COEF) * self.accumulated_delay;

        let remote_recv_time = last_remote_recv_time - zero_time;
        let timing = Timing {
            remote_recv_time_ms: remote_recv_time.as_secs_f64() * 1000.0,
            smoothed_delay_ms: self.smoothed_delay,
        };

        let pos = self
            .history
            .iter()
            .rev()
            .position(|p| p.remote_recv_time_ms <= timing.remote_recv_time_ms)
            .unwrap_or(self.history.len());

        // we expect pos to be 0 more often than not
        self.history.insert(self.history.len() - pos, timing);
    }

    fn update_trendline(&mut self, variation: InterGroupDelayDelta, now: Instant) {
        let trend = self.linear_fit().unwrap_or(self.previous_trend);
        tracing::trace!("Computed trend {:?}", trend);
        log_trendline_estimate!(trend);

        self.detect(trend, variation, now);
    }

    fn linear_fit(&self) -> Option<f64> {
        // Simple linear regression to compute slope.
        assert!(self.history.len() > 2);

        let (sum_x, sum_y) = self.history.iter().fold((0.0, 0.0), |acc, t| {
            (acc.0 + t.remote_recv_time_ms, acc.1 + t.smoothed_delay_ms)
        });

        let avg_x = sum_x / usize_to_f64(self.history.len());
        let avg_y = sum_y / usize_to_f64(self.history.len());

        let (numerator, denomenator) = self.history.iter().fold((0.0, 0.0), |acc, t| {
            let x = t.remote_recv_time_ms;
            let y = t.smoothed_delay_ms;

            let numerator = acc.0 + (x - avg_x) * (y - avg_y);
            let denomenator = acc.1 + (x - avg_x).powi(2);

            (numerator, denomenator)
        });

        if denomenator == 0.0 {
            return None;
        }

        Some(numerator / denomenator)
    }

    fn detect(&mut self, trend: f64, variation: InterGroupDelayDelta, now: Instant) {
        if self.num_delay_variations < 2 {
            self.update_hypothesis(BandwidthUsage::Normal);
            return;
        }

        let modified_trend =
            usize_to_f64(self.num_delay_variations.min(*DELAY_COUNT_RANGE.start()))
                * trend
                * THRESHOLD_GAIN;

        log_trendline_modified_trend!(modified_trend, self.delay_threshold);

        if modified_trend > self.delay_threshold {
            let overuse = if let Some(o) = &mut self.overuse {
                o.time_overusing += variation.send_delta;
                o
            } else {
                let new_overuse = Overuse {
                    count: 0,
                    // Initialize the timer. Assume that we've been
                    // over-using half of the time since the previous
                    // sample.
                    time_overusing: variation.send_delta / 2,
                };
                self.overuse = Some(new_overuse);

                self.overuse.as_mut().unwrap()
            };

            overuse.count += 1;
            tracing::trace!(
                timeoverusing = ?overuse.time_overusing,
                trend,
                previous_trend = self.previous_trend,
                "Trendline Estimator: Maybe overusing"
            );

            if overuse.time_overusing > OVER_USE_TIME_THRESHOLD
                && overuse.count > 1
                && trend > self.previous_trend
            {
                self.overuse = None;

                self.update_hypothesis(BandwidthUsage::Overuse);
            }
        } else if modified_trend < -self.delay_threshold {
            self.overuse = None;
            self.update_hypothesis(BandwidthUsage::Underuse);
        } else {
            self.overuse = None;
            self.update_hypothesis(BandwidthUsage::Normal);
        }

        self.previous_trend = trend;
        self.update_threshold(modified_trend, now);
    }

    fn update_threshold(&mut self, modified_trend: f64, now: Instant) {
        if self.last_threshold_update.is_none() {
            self.last_threshold_update = Some(now);
        }
        let abs_modified_trend = modified_trend.abs();

        if abs_modified_trend > self.delay_threshold + MAX_ADOPT_OFFSET_MS {
            // Avoid adapting the threshold to big latency spikes, caused e.g.,
            // by a sudden capacity drop.
            self.last_threshold_update = Some(now);
            return;
        }

        let k = if abs_modified_trend < self.delay_threshold {
            K_DOWN
        } else {
            K_UP
        };
        let time_delta = u128_to_f64(
            now.saturating_duration_since(
                self.last_threshold_update
                    .expect("last_threshold_update must have been set"),
            )
            .as_millis(),
        );
        self.delay_threshold +=
            k * (abs_modified_trend - self.delay_threshold) * time_delta.min(100.0);
        self.last_threshold_update = Some(now);
        self.delay_threshold = self.delay_threshold.clamp(6.0, 600.0);

        tracing::trace!(
            "Adaptive delay variation threshold changed to: {}",
            self.delay_threshold
        );
    }

    fn update_hypothesis(&mut self, new_hypothesis: BandwidthUsage) {
        if self.hypothesis == new_hypothesis {
            return;
        }

        tracing::trace!("TrendLineEstimator: Setting hypothesis to {new_hypothesis}");
        self.hypothesis = new_hypothesis;
    }
}

/// str0m also stores the arrival `Instant` here (`at`, under `#[allow(unused)]`);
/// nothing reads it, so it is not kept.
#[derive(Debug)]
struct Timing {
    remote_recv_time_ms: f64,
    smoothed_delay_ms: f64,
}

struct Overuse {
    count: usize,
    time_overusing: TimeDelta,
}

#[cfg(test)]
#[path = "trendline_test.rs"]
mod test;
