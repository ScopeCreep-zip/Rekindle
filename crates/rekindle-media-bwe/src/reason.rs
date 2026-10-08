//! Timeout reasons.
//!
//! Copied from str0m `src/lib.rs` (`Reason`), keeping only the variants the
//! estimator and the pacer produce; the DTLS, ICE, SCTP and RTP stream variants
//! belong to str0m's session, which is not copied.

use crate::pacer::PacerReason;

/// The reason for the next timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Reason {
    /// No timeout scheduled.
    ///
    /// The timeout value is in the distant future.
    #[default]
    NotHappening,

    /// Pacer doing things.
    Pacer(PacerReason),

    /// The delay controller of the BWE subsystem.
    BweDelayControl,

    /// The probe controller of the BWE subsystem.
    BweProbeControl,

    /// The probe estimator of the BWE subsystem.
    BweProbeEstimator,
}
