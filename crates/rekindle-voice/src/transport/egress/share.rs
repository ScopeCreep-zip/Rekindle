//! What one kind of media costs on the route (plan E4.3.3): encoder bytes
//! against wire bytes and packets, which the allocator converts between.

use std::time::Instant;

/// Encoder bytes, wire bytes and packets for one kind of media, decayed
/// with a two-second half-life so they follow bitrate changes.
#[derive(Debug, Default)]
pub(super) struct Share {
    media: f64,
    wire: f64,
    packets: f64,
    at: Option<Instant>,
}

impl Share {
    const HALF_LIFE_S: f64 = 2.0;

    pub(super) fn note(&mut self, now: Instant, media: usize, wire: usize) {
        if let Some(at) = self.at {
            let k =
                0.5f64.powf(now.saturating_duration_since(at).as_secs_f64() / Self::HALF_LIFE_S);
            self.media *= k;
            self.wire *= k;
            self.packets *= k;
        }
        self.at = Some(now);
        self.media += as_f64(media);
        self.wire += as_f64(wire);
        self.packets += 1.0;
    }

    /// Encoder bytes per wire byte.
    pub(super) fn ratio(&self) -> Option<f64> {
        (self.wire > 0.0).then(|| self.media / self.wire)
    }

    /// Wire bytes per packet beyond the encoder's.
    pub(super) fn overhead_per_packet(&self) -> Option<f64> {
        (self.packets > 0.0).then(|| (self.wire - self.media) / self.packets)
    }
}

pub(super) fn as_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}
