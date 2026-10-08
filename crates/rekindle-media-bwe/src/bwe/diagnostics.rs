//! What the estimator is basing its estimate on, for the route log line:
//! which of delay, loss, probes and the RTT backoff moved it.

use super::{Bwe, LossControllerState};
use crate::Bitrate;

/// A snapshot of the estimator's inputs and state.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BweDiagnostics {
    /// The acknowledged (delivered) rate the feedback measured.
    pub acked: Option<Bitrate>,
    /// The delay-based estimate.
    pub delay: Option<Bitrate>,
    /// The loss controller's estimate, when it is limiting.
    pub loss: Option<Bitrate>,
    /// Whether the loss controller is limiting: "delay", "decreasing" or
    /// "increasing".
    pub loss_state: &'static str,
    /// RTT backoff cuts so far (missing feedback).
    pub backoff_cuts: u64,
    /// Probe results applied so far.
    pub probes_applied: u64,
}

impl Bwe {
    #[must_use]
    pub fn diagnostics(&self) -> BweDiagnostics {
        let e = &self.estimator;
        let loss = e.loss_controller.loss_based_result();
        BweDiagnostics {
            acked: e.acked_bitrate_estimator.current_estimate(),
            delay: e.delay_controller.last_estimate(),
            loss: loss.bandwidth_estimate,
            loss_state: match loss.state {
                LossControllerState::DelayBased => "delay",
                LossControllerState::Decreasing => "decreasing",
                LossControllerState::Increasing => "increasing",
            },
            backoff_cuts: e.backoff_cuts,
            probes_applied: e.probes_applied,
        }
    }
}
