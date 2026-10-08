use super::*;

use crate::fragment::{fragment_frame, fragment_frame_with_fec, FRAGMENT_PAYLOAD_LIMIT};

use crate::test_mock::test_shape;

fn fragmented(frame_seq: u32, payload: &[u8], keyframe: bool) -> Vec<VideoFragment> {
    fragment_frame(
        test_shape([7u8; STREAM_ID_LEN], frame_seq, keyframe, Codec::Vp9, 0),
        payload,
    )
    .unwrap()
}

#[test]
fn slow_paced_frame_survives_horizon_while_fragments_trickle() {
    // Regression (audit D12): a paced multi-fragment frame whose
    // shards span longer than STALE_FRAME_HORIZON_MS used to be
    // evicted right before its completing fragment arrived —
    // first-fragment anchoring. With last-activity anchoring, a
    // trickle with gaps under the horizon completes.
    let mut r = Reassembler::new();
    let payload = vec![0x44u8; FRAGMENT_PAYLOAD_LIMIT * 3 + 10];
    let frags = fragmented(9, &payload, true);
    assert_eq!(frags.len(), 4);
    let mut t = 0u32;
    for frag in &frags[..3] {
        assert!(r.ingest("alice", frag.clone(), t).unwrap().is_none());
        t += STALE_FRAME_HORIZON_MS - 200; // each gap < horizon
    }
    // Total elapsed ≈ 2.7× the horizon — far past first-fragment
    // eviction, still within last-activity.
    let done = r.ingest("alice", frags[3].clone(), t).unwrap();
    assert!(done.is_some(), "trickled frame must complete");
    assert_eq!(done.unwrap().payload, payload);
}

#[test]
fn abandoned_frame_still_evicts_after_quiet_horizon() {
    // The horizon still works when activity STOPS: a partial with
    // no new fragments for > horizon is evicted on the stream's
    // next ingest.
    let mut r = Reassembler::new();
    let payload = vec![0x55u8; FRAGMENT_PAYLOAD_LIMIT * 2 + 10];
    let frags = fragmented(11, &payload, false);
    assert!(r.ingest("alice", frags[0].clone(), 0).unwrap().is_none());
    // A different frame's fragment arrives long after — sweeping
    // the abandoned partial.
    let other = fragmented(12, &vec![0x66u8; 512], false);
    let _ = r
        .ingest("alice", other[0].clone(), STALE_FRAME_HORIZON_MS + 500)
        .unwrap();
    // Completing the abandoned frame now starts a FRESH partial
    // (the old one is gone), so it does not complete.
    let done = r
        .ingest("alice", frags[1].clone(), STALE_FRAME_HORIZON_MS + 600)
        .unwrap();
    assert!(done.is_none(), "abandoned partial was evicted");
}

#[test]
fn single_fragment_completes_immediately() {
    let mut r = Reassembler::new();
    let frags = fragmented(1, &vec![0xAA; 1024], true);
    let out = r.ingest("alice", frags[0].clone(), 0).unwrap();
    assert!(out.is_some());
    let frame = out.unwrap();
    assert_eq!(frame.frame_seq, 1);
    assert!(frame.keyframe);
    assert!(!frame.recovered_via_fec);
}

#[test]
fn multi_fragment_completes_when_all_arrive() {
    let mut r = Reassembler::new();
    let payload = vec![0x33u8; FRAGMENT_PAYLOAD_LIMIT * 2 + 50];
    let frags = fragmented(5, &payload, false);
    assert_eq!(frags.len(), 3);
    assert!(r.ingest("alice", frags[2].clone(), 100).unwrap().is_none());
    assert!(r.ingest("alice", frags[0].clone(), 100).unwrap().is_none());
    let done = r.ingest("alice", frags[1].clone(), 100).unwrap();
    assert!(done.is_some());
    let frame = done.unwrap();
    assert_eq!(frame.payload, payload);
    assert!(!frame.recovered_via_fec);
}

#[test]
fn duplicate_fragment_does_not_double_count() {
    let mut r = Reassembler::new();
    let frags = fragmented(2, &vec![0x11; FRAGMENT_PAYLOAD_LIMIT * 2 + 1], true);
    assert!(r.ingest("alice", frags[0].clone(), 0).unwrap().is_none());
    assert!(r.ingest("alice", frags[0].clone(), 0).unwrap().is_none());
    assert!(r.ingest("alice", frags[1].clone(), 0).unwrap().is_none());
    let done = r.ingest("alice", frags[2].clone(), 0).unwrap();
    assert!(done.is_some());
}

#[test]
fn stale_partials_are_evicted() {
    let mut r = Reassembler::new();
    let frags = fragmented(1, &vec![0xFF; FRAGMENT_PAYLOAD_LIMIT * 2 + 1], true);
    assert!(r.ingest("alice", frags[0].clone(), 0).unwrap().is_none());
    let later = STALE_FRAME_HORIZON_MS + 1_000;
    let out = r.ingest("alice", frags[1].clone(), later).unwrap();
    assert!(out.is_none());
}

#[test]
fn frag_total_mismatch_rejected() {
    let mut r = Reassembler::new();
    let frags = fragmented(3, &vec![0; FRAGMENT_PAYLOAD_LIMIT * 2 + 1], true);
    let mut tampered = frags[1].clone();
    tampered.frag_total = 5;
    let _ = r.ingest("alice", frags[0].clone(), 0).unwrap();
    let err = r.ingest("alice", tampered, 0).unwrap_err();
    assert!(matches!(err, ReassemblerError::FragTotalMismatch { .. }));
}

#[test]
fn codec_mismatch_rejected() {
    // Phase 2 — a fragment whose codec tag differs from the frame's
    // first-seen tag must be rejected, same posture as the
    // frag_total mismatch above. (Such a fragment also fails its
    // signature check upstream — this guards the buffer invariant
    // independently.)
    let mut r = Reassembler::new();
    let frags = fragmented(4, &vec![0; FRAGMENT_PAYLOAD_LIMIT * 2 + 1], true);
    let mut relabeled = frags[1].clone();
    relabeled.codec = Codec::H264;
    let _ = r.ingest("alice", frags[0].clone(), 0).unwrap();
    let err = r.ingest("alice", relabeled, 0).unwrap_err();
    assert!(matches!(err, ReassemblerError::CodecMismatch));
}

#[test]
fn parity_codec_mismatch_rejected() {
    // Same invariant on the PARITY ingest path.
    let mut r = Reassembler::new();
    let frame = vec![0xEEu8; FRAGMENT_PAYLOAD_LIMIT * 2 + 200];
    let fec = fragment_frame_with_fec(
        test_shape([7u8; STREAM_ID_LEN], 5, true, Codec::Vp9, 0),
        &frame,
        2,
    )
    .unwrap();
    assert!(r.ingest("alice", fec.data[0].clone(), 0).unwrap().is_none());
    let mut relabeled = fec.parity[0].clone();
    relabeled.codec = Codec::Vp8;
    let err = r.ingest_parity("alice", relabeled, 0).unwrap_err();
    assert!(matches!(err, ReassemblerError::CodecMismatch));
}

#[test]
fn fec_recovers_lost_data_shard_via_parity() {
    // 3-data + 2-parity frame. Lose data[1]. Receive remaining
    // data[0], data[2], parity[0]. Reassembler should reconstruct.
    let mut r = Reassembler::new();
    let frame = vec![0xCDu8; FRAGMENT_PAYLOAD_LIMIT * 2 + 200];
    let fec = fragment_frame_with_fec(
        test_shape([5u8; STREAM_ID_LEN], 9, true, Codec::Vp9, 50),
        &frame,
        2,
    )
    .unwrap();
    assert_eq!(fec.data.len(), 3);
    assert_eq!(fec.parity.len(), 2);

    assert!(r.ingest("alice", fec.data[0].clone(), 0).unwrap().is_none());
    assert!(r.ingest("alice", fec.data[2].clone(), 0).unwrap().is_none());
    let done = r.ingest_parity("alice", fec.parity[0].clone(), 0).unwrap();
    assert!(
        done.is_some(),
        "FEC should reconstruct after 2 data + 1 parity"
    );
    let frame_out = done.unwrap();
    assert!(frame_out.recovered_via_fec);
    assert_eq!(frame_out.payload, frame);
}

#[test]
fn fec_completes_early_via_data_plus_parity() {
    // 3 data + 1 parity. After data[0] + data[1] + parity[0]
    // arrive (3 shards total), the frame is already reconstructable
    // via Reed-Solomon. The receiver should NOT wait for data[2].
    // Padding from the equal-shard requirement must be stripped to
    // `frame_len` (verified by the byte-for-byte match below).
    let mut r = Reassembler::new();
    let frame = vec![0xEEu8; FRAGMENT_PAYLOAD_LIMIT * 2 + 13];
    let fec = fragment_frame_with_fec(
        test_shape([6u8; STREAM_ID_LEN], 11, true, Codec::Vp9, 99),
        &frame,
        1,
    )
    .unwrap();
    assert_eq!(fec.data.len(), 3);
    assert_eq!(fec.parity.len(), 1);

    assert!(r
        .ingest_parity("alice", fec.parity[0].clone(), 0)
        .unwrap()
        .is_none());
    assert!(r.ingest("alice", fec.data[0].clone(), 0).unwrap().is_none());
    let done = r.ingest("alice", fec.data[1].clone(), 0).unwrap();
    assert!(
        done.is_some(),
        "3 of 4 shards is enough — must not wait for data[2]"
    );
    let frame_out = done.unwrap();
    assert!(frame_out.recovered_via_fec);
    assert_eq!(
        frame_out.payload, frame,
        "padding must be stripped to frame_len bytes"
    );
}

#[test]
fn fec_fast_path_when_all_data_arrives_first() {
    // Regression for the live "decrypt failed at matching
    // generation" bug: all 3 data shards arrive before any parity
    // (the normal send order), so frame_len is still 0. Under the
    // systematic-code invariant the fast-path concat must yield the
    // EXACT original ciphertext — no FEC padding, because data
    // fragments carry real bytes. Before the fix the last shard was
    // null-padded on the wire and the concat was 1 byte longer,
    // corrupting the AEAD ciphertext.
    let mut r = Reassembler::new();
    let frame = vec![0xCCu8; FRAGMENT_PAYLOAD_LIMIT * 2 + 33];
    let fec = fragment_frame_with_fec(
        test_shape([8u8; STREAM_ID_LEN], 12, true, Codec::Vp9, 1),
        &frame,
        1,
    )
    .unwrap();
    assert!(r.ingest("alice", fec.data[0].clone(), 0).unwrap().is_none());
    assert!(r.ingest("alice", fec.data[1].clone(), 0).unwrap().is_none());
    let done = r.ingest("alice", fec.data[2].clone(), 0).unwrap();
    assert!(done.is_some());
    let frame_out = done.unwrap();
    assert!(!frame_out.recovered_via_fec);
    assert_eq!(frame_out.payload, frame);
}
