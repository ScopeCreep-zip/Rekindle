//! Counts of transport feedback on both ends of each route (plan E4.3 T2
//! diagnostics).
//!
//! Call 2 showed each side receiving 0–10 feedback reports per 5 s where
//! ~20 were due, and nothing said whether the reports were never sent,
//! held up before sending, or lost on the way. This counts, per peer and
//! per stats window: reports built, handed to Veilid (with the time spent
//! waiting to ride a media message and the send time apart) or failed, and
//! reports received from the peer, accepted or dropped.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;

#[derive(Debug, Default)]
struct PeerFeedback {
    built: u64,
    handed: u64,
    failed: u64,
    queued_ms: Vec<u64>,
    send_ms: Vec<u64>,
    accepted: u64,
}

/// One peer's feedback counts over a stats window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerFeedbackWindow {
    pub peer: String,
    /// Reports we built for this peer.
    pub built: u64,
    /// Reports Veilid took.
    pub handed: u64,
    /// Reports Veilid refused.
    pub failed: u64,
    /// Waiting to ride a media message before the send, p50 / p95 / max ms.
    pub queued_ms: (u64, u64, u64),
    /// The `app_message` hand-off itself, p50 / p95 / max ms.
    pub send_ms: (u64, u64, u64),
    /// Reports from this peer about our media, accepted.
    pub accepted: u64,
}

/// A stats window over every peer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedbackWindow {
    pub peers: Vec<PeerFeedbackWindow>,
    /// Reports from keys not on the roster.
    pub unknown_peer: u64,
    /// Reports that did not decode or verify.
    pub rejected: u64,
}

#[derive(Debug, Default)]
struct Counts {
    peers: HashMap<String, PeerFeedback>,
    unknown_peer: u64,
    rejected: u64,
}

#[derive(Debug, Default)]
pub struct FeedbackStats {
    counts: Mutex<Counts>,
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn spread(mut values: Vec<u64>) -> (u64, u64, u64) {
    if values.is_empty() {
        return (0, 0, 0);
    }
    values.sort_unstable();
    let at = |pct: usize| values[(values.len() - 1) * pct / 100];
    (at(50), at(95), values[values.len() - 1])
}

impl FeedbackStats {
    /// A report for `peer` was built.
    pub fn note_built(&self, peer: &str) {
        self.counts
            .lock()
            .peers
            .entry(peer.to_string())
            .or_default()
            .built += 1;
    }

    /// A report for `peer` waited `queued` to ride a message, then took
    /// `send` to hand off; `ok` if Veilid took it.
    pub fn note_handed(&self, peer: &str, queued: Duration, send: Duration, ok: bool) {
        let mut counts = self.counts.lock();
        let entry = counts.peers.entry(peer.to_string()).or_default();
        if ok {
            entry.handed += 1;
        } else {
            entry.failed += 1;
        }
        entry.queued_ms.push(ms(queued));
        entry.send_ms.push(ms(send));
    }

    /// A report from `peer` was accepted.
    pub fn note_accepted(&self, peer: &str) {
        self.counts
            .lock()
            .peers
            .entry(peer.to_string())
            .or_default()
            .accepted += 1;
    }

    /// A report came from a key not on the roster.
    pub fn note_unknown_peer(&self) {
        self.counts.lock().unknown_peer += 1;
    }

    /// A report did not decode or verify.
    pub fn note_rejected(&self) {
        self.counts.lock().rejected += 1;
    }

    /// The counts since the last window, and start a new one.
    pub fn take_window(&self) -> FeedbackWindow {
        let counts = std::mem::take(&mut *self.counts.lock());
        let mut peers: Vec<PeerFeedbackWindow> = counts
            .peers
            .into_iter()
            .map(|(peer, p)| PeerFeedbackWindow {
                peer,
                built: p.built,
                handed: p.handed,
                failed: p.failed,
                queued_ms: spread(p.queued_ms),
                send_ms: spread(p.send_ms),
                accepted: p.accepted,
            })
            .collect();
        peers.sort_by(|a, b| a.peer.cmp(&b.peer));
        FeedbackWindow {
            peers,
            unknown_peer: counts.unknown_peer,
            rejected: counts.rejected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_counts_both_ends_and_resets() {
        let stats = FeedbackStats::default();
        stats.note_built("a");
        stats.note_built("a");
        stats.note_handed(
            "a",
            Duration::from_millis(40),
            Duration::from_millis(3),
            true,
        );
        stats.note_handed(
            "a",
            Duration::from_millis(900),
            Duration::from_millis(5),
            false,
        );
        stats.note_accepted("a");
        stats.note_unknown_peer();
        let w = stats.take_window();
        assert_eq!(w.peers.len(), 1);
        let a = &w.peers[0];
        assert_eq!((a.built, a.handed, a.failed, a.accepted), (2, 1, 1, 1));
        assert_eq!(a.queued_ms, (40, 40, 900));
        assert_eq!(w.unknown_peer, 1);
        assert_eq!(stats.take_window(), FeedbackWindow::default());
    }
}
