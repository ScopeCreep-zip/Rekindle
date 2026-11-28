//! Packet-drop accounting for the two local receive stages.

use std::sync::atomic::{AtomicU64, Ordering};

use rekindle_types::subscription_events::{PacketDropReason, VoiceEvent, VoiceScope};

/// Drops waiting for the next report, and the cumulative local drop count.
#[derive(Default)]
pub struct PacketDrops {
    ingress: AtomicU64,
    receive: AtomicU64,
    total: AtomicU64,
}

impl PacketDrops {
    pub fn record_ingress(&self) {
        self.ingress.fetch_add(1, Ordering::Relaxed);
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_receive(&self) {
        self.receive.fetch_add(1, Ordering::Relaxed);
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    #[must_use]
    pub fn total_drops(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Clear the counters when the identity unlock scope ends.
    pub fn reset(&self) {
        self.ingress.store(0, Ordering::Relaxed);
        self.receive.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
    }

    /// Drain this interval's counters, then build events for the active scope.
    pub fn drain(&self, scope: Option<VoiceScope>) -> impl Iterator<Item = VoiceEvent> {
        let counts = [
            (
                PacketDropReason::Ingress,
                self.ingress.swap(0, Ordering::Relaxed),
            ),
            (
                PacketDropReason::ReceiveLoop,
                self.receive.swap(0, Ordering::Relaxed),
            ),
        ];
        counts.into_iter().filter_map(move |(reason, count)| {
            let scope = scope.as_ref()?;
            (count != 0).then(|| VoiceEvent::packets_dropped(scope.clone(), reason, count))
        })
    }
}

#[cfg(test)]
#[path = "packet_drops_tests.rs"]
mod tests;
