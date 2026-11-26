use std::time::Duration;

use super::{RateControl, Signal, State};

#[path = "rate_control_test_state.rs"]
mod state;

#[path = "rate_control_test_rate_controller.rs"]
mod rate_controller;

fn duration_ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}
