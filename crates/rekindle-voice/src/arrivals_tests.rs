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
