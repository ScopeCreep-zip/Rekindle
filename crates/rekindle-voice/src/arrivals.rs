//! Receive-side arrival record for transport feedback (plan E4.3.2).
//!
//! The dispatch thread notes `(peer, transport_seq, arrival)` for every
//! sequenced media datagram the moment it arrives — later stages (the
//! ingress worker, the receive loop) would add their own queueing to the
//! delay the estimator measures. The receive loop drains it into one
//! RFC 8888-shaped report per peer every feedback interval
//! (`evidence/e4-3-bandwidth-owner-design.md` #4).
//!
//! The outer sequence number is outside the payload signature (one signed
//! packet goes to every peer), so a party holding our route could replay
//! a peer's signed packet under a new sequence number and fake an arrival
//! for it (plan E4.3.3). Each kind is recorded only once fresh: a voice
//! packet through a replay window over its own signed sequence
//! ([`ArrivalLedger::record_voice`]); an envelope by its signature, seen
//! once ([`ArrivalLedger::record_signed`]); padding signs the sequence
//! number itself. A sequence number keeps its first arrival, so a replay
//! under the same number cannot move it.
//!
//! Arrivals are kept after they are reported, for [`BACK_WINDOW`], as
//! libwebrtc's feedback generator keeps them
//! (`transport_sequence_number_feedback_generator.cc`): a packet that
//! arrives after a report already called it lost moves the next report's
//! start back to it, so it is reported received (RFC 8888 §3 overlapping
//! reports). Veilid's RPC workers deliver out of order and release bursts;
//! clearing at each report turned every such late arrival into permanent
//! loss (call 1: 6.5 % feedback loss against 0 % voice loss,
//! `evidence/e4-3-transport-on-veilid.md`).
//!
//! Every kind is recorded on the dispatch thread, before any queue, as
//! libwebrtc hands each packet's arrival to feedback before delivering it
//! to a stream (`call/call.cc` `NotifyBweOfReceivedPacket`): an arrival
//! recorded behind a queue can land after a report has already counted
//! its sequence number lost.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::replay_window::CtrWindow;
use rekindle_codec::capnp_codec::transport_feedback::{MAX_ARRIVAL_OFFSET, NOT_RECEIVED};

/// Most sequence numbers one report covers. A peer whose report would
/// span more (a long stall, or a sequence jump) starts again from its
/// newest packets: the estimator needs recent arrivals, not a backlog.
pub const MAX_REPORT_SPAN: u32 = 1_000;

/// How long an arrival is kept after newer ones, so a late packet can still
/// be reported: libwebrtc's `kBackWindow`.
pub const BACK_WINDOW: Duration = Duration::from_millis(500);

/// One peer's recent arrivals.
#[derive(Debug, Default)]
struct PeerArrivals {
    /// The newest sequence number unwrapped so far: sequence numbers are
    /// kept as `i64` so ordering survives the u32 wrap (libwebrtc's
    /// `SeqNumUnwrapper`).
    last_unwrapped: Option<i64>,
    /// First sequence number the next report covers.
    window_start: Option<i64>,
    /// Arrivals within [`BACK_WINDOW`] of the newest, reported or not.
    arrivals: BTreeMap<i64, Instant>,
    /// Whether anything arrived since the last report.
    unreported: bool,
    /// Voice sequence numbers already counted (RFC 3711 §3.3.2 window).
    voice_seen: CtrWindow,
    /// Envelope signatures already counted, oldest first (bounded).
    signed_seen: std::collections::VecDeque<[u8; 16]>,
    signed_set: std::collections::HashSet<[u8; 16]>,
}

impl PeerArrivals {
    fn unwrap(&mut self, seq: u32) -> i64 {
        let unwrapped = match self.last_unwrapped {
            None => i64::from(seq),
            Some(last) => {
                let low = u32::try_from(last & i64::from(u32::MAX)).unwrap_or_default();
                // The signed distance from the last one: a u32 delta read
                // as i32 (libwebrtc's unwrapper).
                last + i64::from(seq.wrapping_sub(low).cast_signed())
            }
        };
        self.last_unwrapped = Some(self.last_unwrapped.map_or(unwrapped, |l| l.max(unwrapped)));
        unwrapped
    }

    fn record(&mut self, transport_seq: u32, at: Instant) {
        let seq = self.unwrap(transport_seq);
        // A sequence number keeps its first arrival; a duplicate changes
        // nothing and asks for no report.
        if self.arrivals.contains_key(&seq) {
            return;
        }
        // Drop arrivals older than the back window behind this one
        // (`MaybeCullOldPackets`).
        if let Some(cutoff) = at.checked_sub(BACK_WINDOW) {
            self.arrivals.retain(|_, t| *t >= cutoff);
        }
        // A late packet moves the next report back to include it.
        self.window_start = Some(self.window_start.map_or(seq, |w| w.min(seq)));
        self.arrivals.insert(seq, at);
        if let Some((&oldest, _)) = self.arrivals.first_key_value() {
            if self.window_start.is_some_and(|w| w < oldest) {
                self.window_start = Some(oldest);
            }
        }
        self.unreported = true;
    }

    fn take_report(&mut self, now: Instant) -> Option<ReportBody> {
        if !self.unreported {
            return None;
        }
        let (&newest, _) = self.arrivals.last_key_value()?;
        let start = self
            .window_start?
            .max(newest + 1 - i64::from(MAX_REPORT_SPAN));
        let arrivals = (start..=newest)
            .map(|seq| {
                self.arrivals.get(&seq).map_or(NOT_RECEIVED, |at| {
                    let ticks = now.saturating_duration_since(*at).as_micros() * 1024 / 1_000_000;
                    u16::try_from(ticks)
                        .unwrap_or(MAX_ARRIVAL_OFFSET)
                        .min(MAX_ARRIVAL_OFFSET)
                })
            })
            .collect();
        // Reported arrivals stay until the back window culls them, in case
        // a reordering needs them again.
        self.window_start = Some(newest + 1);
        self.unreported = false;
        Some(ReportBody {
            begin_seq: u32::try_from(start & i64::from(u32::MAX)).unwrap_or_default(),
            arrivals,
        })
    }
}

/// Envelope signatures remembered per peer for the replay check: well over
/// a feedback interval of video at any rate the route carries.
const SIGNED_SEEN_MAX: usize = 4_096;

/// A report's content before it is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportBody {
    pub begin_seq: u32,
    pub arrivals: Vec<u16>,
}

/// Arrivals per sending peer (pseudonym or identity key, hex).
#[derive(Debug)]
pub struct ArrivalLedger {
    peers: Mutex<HashMap<String, PeerArrivals>>,
    /// Origin of the report clock: monotonic, and stable across receive
    /// loop restarts (device swaps), so successive reports agree.
    origin: Instant,
}

impl Default for ArrivalLedger {
    fn default() -> Self {
        Self {
            peers: Mutex::default(),
            origin: Instant::now(),
        }
    }
}

impl ArrivalLedger {
    /// Note that `peer`'s datagram `transport_seq` arrived at `at`.
    pub fn record(&self, peer: &str, transport_seq: u32, at: Instant) {
        let mut peers = self.peers.lock();
        peers
            .entry(peer.to_string())
            .or_default()
            .record(transport_seq, at);
    }

    /// Note a signed envelope's arrival unless the same signature was
    /// counted before (a replay under a new route sequence number).
    pub fn record_signed(&self, peer: &str, signature: &[u8], transport_seq: u32, at: Instant) {
        let mut key = [0u8; 16];
        let n = signature.len().min(16);
        key[..n].copy_from_slice(&signature[..n]);
        let fresh = {
            let mut peers = self.peers.lock();
            let entry = peers.entry(peer.to_string()).or_default();
            let fresh = entry.signed_set.insert(key);
            if fresh {
                entry.signed_seen.push_back(key);
                if entry.signed_seen.len() > SIGNED_SEEN_MAX {
                    if let Some(old) = entry.signed_seen.pop_front() {
                        entry.signed_set.remove(&old);
                    }
                }
            }
            fresh
        };
        if fresh {
            self.record(peer, transport_seq, at);
        }
    }

    /// Note a voice packet's arrival unless its signed `voice_sequence`
    /// from `peer` was counted before.
    pub fn record_voice(&self, peer: &str, voice_sequence: u32, transport_seq: u32, at: Instant) {
        let fresh = self
            .peers
            .lock()
            .entry(peer.to_string())
            .or_default()
            .voice_seen
            .check_and_insert(u64::from(voice_sequence));
        if fresh {
            self.record(peer, transport_seq, at);
        }
    }

    /// Build `peer`'s report as of `now`: every sequence number from the
    /// report window's start through the newest arrival, each received
    /// (with its offset before `now` in 1/1024 s) or not. The window starts
    /// after the last report, or earlier when a packet arrived late. `None`
    /// when nothing arrived since the last report.
    pub fn take_report(&self, peer: &str, now: Instant) -> Option<ReportBody> {
        let mut peers = self.peers.lock();
        peers.get_mut(peer)?.take_report(now)
    }

    /// `now` on the report clock, milliseconds.
    #[must_use]
    pub fn report_time_ms(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.origin).as_millis()).unwrap_or(u64::MAX)
    }

    /// Peers with arrivals waiting to be reported.
    #[must_use]
    pub fn peers_with_arrivals(&self) -> Vec<String> {
        self.peers
            .lock()
            .iter()
            .filter(|(_, a)| a.unreported)
            .map(|(k, _)| k.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn report_covers_gaps_and_offsets() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", 10, t0);
        ledger.record("a", 12, t0 + Duration::from_millis(500));
        let now = t0 + Duration::from_secs(1);
        let report = ledger.take_report("a", now).unwrap();
        assert_eq!(report.begin_seq, 10);
        assert_eq!(report.arrivals, vec![1024, NOT_RECEIVED, 512]);
        // Nothing new: no report; the next covers from 13.
        assert!(ledger.take_report("a", now).is_none());
        ledger.record("a", 13, now);
        assert_eq!(ledger.take_report("a", now).unwrap().begin_seq, 13);
    }

    #[test]
    fn reports_survive_the_sequence_wrap() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", u32::MAX, t0);
        ledger.record("a", 0, t0);
        let report = ledger.take_report("a", t0).unwrap();
        assert_eq!(report.begin_seq, u32::MAX);
        assert_eq!(report.arrivals.len(), 2);
    }

    #[test]
    fn a_long_gap_reports_only_the_newest_span() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", 0, t0);
        ledger.record("a", 5_000, t0);
        let report = ledger.take_report("a", t0).unwrap();
        assert_eq!(report.arrivals.len(), MAX_REPORT_SPAN as usize);
        assert_eq!(report.begin_seq, 5_000 + 1 - MAX_REPORT_SPAN);
    }

    #[test]
    fn a_replayed_voice_packet_does_not_count_again() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record_voice("a", 7, 100, t0);
        // The same signed packet replayed under a new route sequence.
        ledger.record_voice("a", 7, 101, t0);
        let report = ledger.take_report("a", t0).unwrap();
        assert_eq!(report.arrivals.len(), 1);
    }

    #[test]
    fn a_replayed_envelope_does_not_count_again() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record_signed("a", &[7u8; 64], 10, t0);
        ledger.record_signed("a", &[7u8; 64], 11, t0);
        ledger.record_signed("a", &[8u8; 64], 12, t0);
        let report = ledger.take_report("a", t0).unwrap();
        assert_eq!(report.begin_seq, 10);
        assert_eq!(report.arrivals.len(), 3);
        assert_eq!(report.arrivals[1], NOT_RECEIVED, "seq 11 was the replay");
    }

    #[test]
    fn a_sequence_number_keeps_its_first_arrival() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", 5, t0);
        ledger.record("a", 5, t0 + Duration::from_millis(500));
        let report = ledger
            .take_report("a", t0 + Duration::from_secs(1))
            .unwrap();
        assert_eq!(report.arrivals, vec![1024]);
    }

    #[test]
    fn duplicates_of_reported_packets_ask_for_nothing() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", 100, t0);
        ledger.take_report("a", t0).unwrap();
        ledger.record("a", 100, t0 + Duration::from_millis(5));
        assert!(ledger.take_report("a", t0).is_none());
    }

    #[test]
    fn a_packet_behind_the_first_report_is_reported_late() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", 100, t0);
        ledger.take_report("a", t0).unwrap();
        ledger.record("a", 99, t0);
        let report = ledger.take_report("a", t0).unwrap();
        assert_eq!(report.begin_seq, 99);
        assert_ne!(report.arrivals[0], NOT_RECEIVED);
    }

    #[test]
    fn a_late_packet_is_reported_received_after_all() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", 10, t0);
        ledger.record("a", 12, t0 + Duration::from_millis(20));
        let first = ledger
            .take_report("a", t0 + Duration::from_millis(50))
            .unwrap();
        assert_eq!(first.arrivals[1], NOT_RECEIVED, "11 had not arrived");
        // 11 arrives late, after the report that called it lost.
        ledger.record("a", 11, t0 + Duration::from_millis(100));
        ledger.record("a", 13, t0 + Duration::from_millis(110));
        let second = ledger
            .take_report("a", t0 + Duration::from_millis(120))
            .unwrap();
        assert_eq!(
            second.begin_seq, 11,
            "the window moved back to the late packet"
        );
        assert_eq!(second.arrivals.len(), 3);
        assert_ne!(second.arrivals[0], NOT_RECEIVED, "11 reported received");
    }

    #[test]
    fn reordering_inside_the_back_window_loses_nothing() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        let mut lost = 0;
        // Arrivals in bursts, each burst reversed, a report between bursts.
        for burst in 0..10_u32 {
            let at = t0 + Duration::from_millis(u64::from(burst) * 40);
            for seq in (burst * 4..burst * 4 + 4).rev() {
                if seq % 4 != 0 || burst == 0 {
                    ledger.record("a", seq, at);
                }
            }
            // The first of each burst arrives after the report.
            let report = ledger
                .take_report("a", at + Duration::from_millis(1))
                .unwrap();
            if burst > 0 {
                ledger.record("a", burst * 4, at + Duration::from_millis(2));
            }
            lost += report
                .arrivals
                .iter()
                .filter(|a| **a == NOT_RECEIVED)
                .count();
        }
        let last = ledger
            .take_report("a", t0 + Duration::from_secs(1))
            .unwrap();
        assert!(last.arrivals.iter().all(|a| *a != NOT_RECEIVED));
        assert!(lost > 0, "reports did call them lost at the time");
    }

    #[test]
    fn an_arrival_older_than_the_back_window_is_gone() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", 1, t0);
        ledger.take_report("a", t0).unwrap();
        ledger.record("a", 3, t0 + BACK_WINDOW + Duration::from_millis(100));
        let report = ledger
            .take_report("a", t0 + BACK_WINDOW + Duration::from_millis(100))
            .unwrap();
        assert_eq!(
            report.begin_seq, 3,
            "1 was culled; the window starts at what is kept"
        );
        assert_eq!(report.arrivals.len(), 1);
    }
}
