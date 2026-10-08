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
//! Every kind is recorded on the dispatch thread, before any queue, as
//! libwebrtc hands each packet's arrival to feedback before delivering it
//! to a stream (`call/call.cc` `NotifyBweOfReceivedPacket`): an arrival
//! recorded behind a queue can land after a report has already counted
//! its sequence number lost.

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use parking_lot::Mutex;

use crate::replay_window::CtrWindow;
use rekindle_codec::capnp_codec::transport_feedback::{MAX_ARRIVAL_OFFSET, NOT_RECEIVED};

/// Most sequence numbers one report covers. A peer whose report would
/// span more (a long stall, or a sequence jump) starts again from its
/// newest packets: the estimator needs recent arrivals, not a backlog.
pub const MAX_REPORT_SPAN: u32 = 1_000;

/// One peer's arrivals since its last report.
#[derive(Debug, Default)]
struct PeerArrivals {
    /// First sequence number the next report covers.
    next_report_seq: Option<u32>,
    /// Arrivals not yet reported, keyed by sequence number relative to
    /// `next_report_seq` (so ordering survives the u32 wrap).
    pending: BTreeMap<u32, Instant>,
    /// Voice sequence numbers already counted (RFC 3711 §3.3.2 window).
    voice_seen: CtrWindow,
    /// Envelope signatures already counted, oldest first (bounded).
    signed_seen: std::collections::VecDeque<[u8; 16]>,
    signed_set: std::collections::HashSet<[u8; 16]>,
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
        let entry = peers.entry(peer.to_string()).or_default();
        let base = *entry.next_report_seq.get_or_insert(transport_seq);
        let offset = transport_seq.wrapping_sub(base);
        // Before the window (a late duplicate of something already
        // reported) is dropped: its loss was already reported.
        if offset < u32::MAX / 2 {
            entry.pending.entry(offset).or_insert(at);
        }
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
    /// last report through the newest arrival, each received (with its
    /// offset before `now` in 1/1024 s) or not. `None` when nothing new
    /// arrived.
    pub fn take_report(&self, peer: &str, now: Instant) -> Option<ReportBody> {
        let mut peers = self.peers.lock();
        let entry = peers.get_mut(peer)?;
        let base = entry.next_report_seq?;
        let (&last, _) = entry.pending.last_key_value()?;
        let (begin_offset, span) = if last >= MAX_REPORT_SPAN {
            (last + 1 - MAX_REPORT_SPAN, MAX_REPORT_SPAN)
        } else {
            (0, last + 1)
        };
        let arrivals = (begin_offset..begin_offset + span)
            .map(|offset| {
                entry.pending.get(&offset).map_or(NOT_RECEIVED, |at| {
                    let ticks = now.saturating_duration_since(*at).as_micros() * 1024 / 1_000_000;
                    u16::try_from(ticks)
                        .unwrap_or(MAX_ARRIVAL_OFFSET)
                        .min(MAX_ARRIVAL_OFFSET)
                })
            })
            .collect();
        entry.next_report_seq = Some(base.wrapping_add(last + 1));
        entry.pending.clear();
        Some(ReportBody {
            begin_seq: base.wrapping_add(begin_offset),
            arrivals,
        })
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
            .filter(|(_, a)| !a.pending.is_empty())
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
    fn late_duplicates_of_reported_packets_are_ignored() {
        let ledger = ArrivalLedger::default();
        let t0 = Instant::now();
        ledger.record("a", 100, t0);
        ledger.take_report("a", t0).unwrap();
        ledger.record("a", 99, t0);
        assert!(ledger.take_report("a", t0).is_none());
    }
}
