//! Idle tier state machine — controls render rate, tick rate, poll frequency.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdleTier {
    /// 0–5s since last input. 30fps render, 4Hz tick, 2s status poll.
    Active,
    /// 5–30s. 4fps render, 1Hz tick, 10s status poll.
    Idle,
    /// 30s–5min. 1fps render, 0.5Hz tick, 60s status poll.
    DeepIdle,
    /// 5min+. 0.2fps render, no tick, no status poll.
    Suspended,
}

impl IdleTier {
    pub fn render_interval(&self) -> Duration {
        match self {
            Self::Active => Duration::from_millis(33),
            Self::Idle => Duration::from_millis(250),
            Self::DeepIdle => Duration::from_secs(1),
            Self::Suspended => Duration::from_secs(5),
        }
    }

    pub fn tick_interval(&self) -> Duration {
        match self {
            Self::Active => Duration::from_millis(250),
            Self::Idle => Duration::from_secs(1),
            Self::DeepIdle => Duration::from_secs(2),
            Self::Suspended => Duration::from_secs(5),
        }
    }

    pub fn status_poll_interval(&self) -> Duration {
        match self {
            Self::Active => Duration::from_secs(2),
            Self::Idle => Duration::from_secs(10),
            Self::DeepIdle => Duration::from_secs(60),
            Self::Suspended => Duration::from_secs(300),
        }
    }

    pub fn should_poll_status(&self) -> bool {
        !matches!(self, Self::Suspended)
    }

    pub fn should_full_refresh(&self) -> bool {
        matches!(self, Self::Active | Self::Idle)
    }

    /// Compute the tier from elapsed time since last user input.
    pub fn from_elapsed(last_input: Instant, now: Instant) -> Self {
        let elapsed = now.duration_since(last_input);
        if elapsed < Duration::from_secs(5) {
            Self::Active
        } else if elapsed < Duration::from_secs(30) {
            Self::Idle
        } else if elapsed < Duration::from_secs(300) {
            Self::DeepIdle
        } else {
            Self::Suspended
        }
    }
}

/// Returns `true` if the interval needs resetting (period changed).
pub fn adjust_interval(
    interval: &mut tokio::time::Interval,
    new_period: Duration,
) -> bool {
    if interval.period() != new_period {
        *interval = tokio::time::interval(new_period);
        true
    } else {
        false
    }
}
