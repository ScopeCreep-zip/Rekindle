use super::*;

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn metrics() -> ReceptionMetrics {
    ReceptionMetrics {
        loss_rate_q8: 13,
        discard_rate_q8: 4,
        burst_density_q8: 90,
        gap_density_q8: 1,
        burst_duration_ms: 120,
        gap_duration_ms: 4200,
        jitter_ms: 27,
        delay_p50_ms: 12,
        delay_p95_ms: 48,
        packets_expected: 500,
        packets_received: 487,
    }
}

fn signed_report() -> VoiceReceiverReport {
    let sk = key();
    let mut r = VoiceReceiverReport::new(
        sk.verifying_key().to_bytes().to_vec(),
        1234,
        metrics(),
        9_000,
        15,
        60,
    );
    r.sign(&sk);
    r
}

fn packet(seq: u32, ts: u64) -> VoicePacket {
    VoicePacket {
        sender_key: vec![0; 32],
        sequence: seq,
        timestamp: ts,
        sframe: vec![0; 8],
        sig: Vec::new(),
    }
}

#[test]
fn round_trips_through_the_wire_with_the_tag_stripped() {
    let wire = signed_report().to_wire().unwrap();
    assert_eq!(wire[0], RECEIVER_REPORT_TAG, "tagged for the dispatch loop");
    let back = VoiceReceiverReport::from_wire(&wire[1..]).unwrap();
    assert_eq!(back.metrics, metrics());
    assert_eq!(back.highest_seq, 1234);
    assert_eq!(back.jb_nominal_ms, 60);
}

#[test]
fn unsigned_report_is_rejected() {
    let r = VoiceReceiverReport::new(
        key().verifying_key().to_bytes().to_vec(),
        1,
        metrics(),
        0,
        0,
        40,
    );
    let wire = r.to_wire().unwrap();
    assert!(
        VoiceReceiverReport::from_wire(&wire[1..]).is_err(),
        "an unsigned report must not be actionable"
    );
}

#[test]
fn every_actionable_field_is_covered_by_the_signature() {
    // The DoS this prevents: rewrite a metric in flight and have
    // the sender still believe it. Each mutation must break verify.
    let base = signed_report();
    let mut cases: Vec<(&str, VoiceReceiverReport)> = Vec::new();

    let mut loss = base.clone();
    loss.metrics.loss_rate_q8 = 255;
    cases.push(("loss_rate_q8", loss));

    let mut discard = base.clone();
    discard.metrics.discard_rate_q8 = 255;
    cases.push(("discard_rate_q8", discard));

    let mut jitter = base.clone();
    jitter.metrics.jitter_ms = 9_999;
    cases.push(("jitter_ms", jitter));

    let mut burst = base.clone();
    burst.metrics.burst_density_q8 = 255;
    cases.push(("burst_density_q8", burst));

    let mut gap = base.clone();
    gap.metrics.gap_density_q8 = 255;
    cases.push(("gap_density_q8", gap));

    let mut burst_ms = base.clone();
    burst_ms.metrics.burst_duration_ms = 60_000;
    cases.push(("burst_duration_ms", burst_ms));

    let mut gap_ms = base.clone();
    gap_ms.metrics.gap_duration_ms = 1;
    cases.push(("gap_duration_ms", gap_ms));

    let mut expected = base.clone();
    expected.metrics.packets_expected = 1;
    cases.push(("packets_expected", expected));

    let mut received = base.clone();
    received.metrics.packets_received = 1;
    cases.push(("packets_received", received));

    let mut seq = base.clone();
    seq.highest_seq = 0;
    cases.push(("highest_seq", seq));

    let mut lsr = base.clone();
    lsr.lsr_ms = 1;
    cases.push(("lsr_ms", lsr));

    let mut dlsr = base.clone();
    dlsr.dlsr_ms = 5_000;
    cases.push(("dlsr_ms", dlsr));

    let mut jb = base.clone();
    jb.jb_nominal_ms = 120;
    cases.push(("jb_nominal_ms", jb));

    for (field, tampered) in cases {
        let wire = tampered.to_wire().unwrap();
        assert!(
            VoiceReceiverReport::from_wire(&wire[1..]).is_err(),
            "tampering with {field} must invalidate the signature"
        );
    }
}

#[test]
fn report_signed_by_one_peer_cannot_be_attributed_to_another() {
    let mut r = signed_report();
    // Swap in a different reporter key, keeping the signature.
    r.reporter_key = SigningKey::from_bytes(&[9u8; 32])
        .verifying_key()
        .to_bytes()
        .to_vec();
    let wire = r.to_wire().unwrap();
    assert!(VoiceReceiverReport::from_wire(&wire[1..]).is_err());
}

#[test]
fn packet_signature_cannot_be_replayed_as_a_report_signature() {
    // Both sign with the same identity key; only the domain tag
    // separates them.
    let sk = key();
    let p = packet(1, 20);
    let packet_sig = sk.sign(&p.signing_bytes());
    let mut r = VoiceReceiverReport::new(
        sk.verifying_key().to_bytes().to_vec(),
        1,
        metrics(),
        20,
        0,
        40,
    );
    r.signature = packet_sig.to_bytes().to_vec();
    let wire = r.to_wire().unwrap();
    assert!(VoiceReceiverReport::from_wire(&wire[1..]).is_err());
}

#[test]
fn rtt_subtracts_the_receivers_own_delay() {
    let r = signed_report(); // lsr 9_000, dlsr 15
                             // Sender's clock reads 9_100: 100 ms round trip, of which 15
                             // was the receiver sitting on the report.
    assert_eq!(r.round_trip_ms(9_100), Some(85));
}

#[test]
fn impossible_rtt_is_refused_not_clamped() {
    let r = signed_report(); // lsr 9_000, dlsr 15
                             // An lsr from the future.
    assert_eq!(r.round_trip_ms(8_000), None);
    // A dlsr longer than the packet has existed.
    assert_eq!(r.round_trip_ms(9_010), None);
}

#[test]
fn echo_tracks_the_newest_packet_and_ignores_reordering() {
    let mut echo = SenderEcho::default();
    assert_eq!(echo.echo_at(500), None, "nothing to echo yet");

    echo.observe(&packet(10, 200), 1_000);
    assert_eq!(echo.echo_at(1_050), Some((200, 50)));
    assert_eq!(echo.highest_seq(), 10);

    // A reordered older packet must not drag the echo back — doing
    // so would inflate DLSR by the reorder distance.
    echo.observe(&packet(9, 180), 1_060);
    assert_eq!(echo.echo_at(1_050), Some((200, 50)));
    assert_eq!(echo.highest_seq(), 10);

    echo.observe(&packet(11, 220), 1_070);
    assert_eq!(echo.echo_at(1_100), Some((220, 30)));
    assert_eq!(echo.highest_seq(), 11);
}
