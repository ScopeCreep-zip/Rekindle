//! The merged connection-quality state of one voice session, and the
//! projection of the send half's measurement onto the Tier-1 event
//! payload. One definition for every host that maps
//! [`VoiceSessionEvent`]s to `VoiceEvent`s (the Host's voice adapter and,
//! until plan E4.1 S7 deletes it, the desktop's).

use rekindle_types::subscription_events::{LinkMeasurement, VoiceEvent, VoiceScope};

use crate::session_deps::{SendLinkStats, VoiceSessionEvent};

/// Last-known halves of the merged `ConnectionQuality` event. The send
/// loop reports `quality` and the receive loop reports jitter drops on
/// independent cadences; each emission carries the full picture from
/// this cache.
#[derive(Debug, Clone)]
pub struct VoiceQualityCache {
    pub quality: String,
    pub rx_overflow_drops: u64,
    pub rx_late_drops: u64,
    pub rx_key_drops: u64,
    /// The send half's end-to-end measurement, from receiver reports.
    /// `None` until a peer reports.
    pub link: Option<LinkMeasurement>,
}

impl Default for VoiceQualityCache {
    fn default() -> Self {
        Self {
            quality: "good".to_string(),
            rx_overflow_drops: 0,
            rx_late_drops: 0,
            rx_key_drops: 0,
            link: None,
        }
    }
}

impl VoiceQualityCache {
    /// Fold a `ConnectionQuality` or `ReceiveStats` event into the cache.
    /// Returns `false` (and leaves the cache alone) for every other event.
    pub fn fold(&mut self, event: &VoiceSessionEvent) -> bool {
        match event {
            VoiceSessionEvent::ConnectionQuality { quality, link } => {
                self.quality.clone_from(quality);
                // A window with no report keeps the last numbers, or the
                // UI blinks empty between reports.
                if let Some(stats) = link {
                    self.link = Some(project_link(stats));
                }
                true
            }
            VoiceSessionEvent::ReceiveStats {
                rx_overflow_drops,
                rx_late_drops,
                rx_key_drops,
            } => {
                self.rx_overflow_drops = *rx_overflow_drops;
                self.rx_late_drops = *rx_late_drops;
                self.rx_key_drops = *rx_key_drops;
                true
            }
            _ => false,
        }
    }

    /// The merged event for the session on `scope`; `ingress_drops` is
    /// the host's inbound drop count.
    #[must_use]
    pub fn connection_quality(&self, scope: VoiceScope, ingress_drops: u64) -> VoiceEvent {
        VoiceEvent::ConnectionQuality {
            scope,
            quality: self.quality.clone(),
            rx_overflow_drops: self.rx_overflow_drops,
            rx_late_drops: self.rx_late_drops,
            rx_key_drops: self.rx_key_drops,
            ingress_drops,
            link: self.link,
        }
    }
}

/// Project the crate's measurement types onto the flat Tier-1 event
/// payload. The one place the two representations meet — see
/// [`LinkMeasurement`]'s note on why they are separate.
fn project_link(stats: &SendLinkStats) -> LinkMeasurement {
    LinkMeasurement {
        loss_q8: stats.metrics.loss_rate_q8,
        discard_q8: stats.metrics.discard_rate_q8,
        jitter_ms: stats.metrics.jitter_ms,
        rtt_ms: stats.rtt_ms,
        r_factor: stats.score.r_factor,
        mos_lq: stats.score.mos_lq,
        mos_cq: stats.score.mos_cq,
        bitrate_bps: stats.bitrate_bps,
        one_way_p50_ms: stats.route.map(|r| r.one_way_p50_ms),
        one_way_p95_ms: stats.route.map(|r| r.one_way_p95_ms),
        mouth_to_ear_ms: stats.route.map(|r| r.mouth_to_ear_ms),
        outside_g114: stats.route.is_some_and(|r| r.outside_g114),
    }
}
