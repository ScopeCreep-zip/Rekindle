use crate::Bitrate;

const PACING_FACTOR: f64 = 1.1;

/// Target padding rate when media is active. This maintains NAT bindings, RTX state,
/// and allows ALR periodic probes to discover higher bandwidth.
const PADDING_TARGET: Bitrate = Bitrate::bps(50_000);

/// The rates [`PacerControl::calculate`] derives.
pub struct PacingResult {
    /// The bitrate at which the pacer may emit padding **when there is no media queued**.
    ///
    /// This value is intentionally an *absolute* target used by the pacer, not a “delta to add
    /// on top of current media”. In practice the **effective padding** over time is the
    /// difference between this target and the media actually sent, because padding is only used
    /// to fill gaps (empty queues), not to continuously top-up while media is flowing.
    pub padding_rate: Bitrate,
    /// The rate the pacer paces media at (estimate × 1.1, at least the padding rate).
    pub pacing_rate: Bitrate,
}

/// Controls the pacing and padding rates.
#[derive(Debug, Default)]
pub struct PacerControl {}

impl PacerControl {
    /// A pacer control.
    pub fn new() -> Self {
        Self {}
    }

    /// The padding and pacing rates for the current estimate.
    ///
    /// str0m takes `&self` here without reading it; it is an associated function
    /// here because the workspace lints reject an unused `self`.
    pub fn calculate(has_active_media: bool, estimate: Bitrate, is_overuse: bool) -> PacingResult {
        // ALR periodic probes handle bandwidth discovery, not continuous padding.
        let padding_rate = if is_overuse {
            // No padding during overuse
            Bitrate::ZERO
        } else if has_active_media {
            // Pad to 50 kbps to maintain NAT bindings and RTX state
            PADDING_TARGET
        } else {
            // No padding when no media is being sent
            Bitrate::ZERO
        };

        // Set pacing rate to smooth out media transmission (burst avoidance).
        // Must be at least the current BWE estimate * factor, but also high enough
        // to allow the padding we want to send.
        let min_pacing_rate = estimate * PACING_FACTOR;
        let pacing_rate = min_pacing_rate.max(padding_rate);

        PacingResult {
            padding_rate,
            pacing_rate,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn padding_enabled_with_active_media() {
        let estimate = Bitrate::kbps(1_000);

        let r = PacerControl::calculate(true, estimate, false);

        assert_eq!(r.padding_rate, PADDING_TARGET);
    }

    #[test]
    fn no_padding_without_active_media() {
        let estimate = Bitrate::kbps(1_000);

        let r = PacerControl::calculate(false, estimate, false);

        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }

    #[test]
    fn overuse_suppresses_padding() {
        let estimate = Bitrate::mbps(40);

        let r = PacerControl::calculate(true, estimate, true);
        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }
}
