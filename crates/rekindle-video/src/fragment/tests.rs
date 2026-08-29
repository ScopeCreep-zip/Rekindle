use super::*;

fn test_shape(
    stream_id: [u8; STREAM_ID_LEN],
    frame_seq: u32,
    keyframe: bool,
    codec: Codec,
    timestamp: u32,
) -> FrameShape {
    FrameShape {
        stream_id,
        frame_seq,
        keyframe,
        codec,
        timestamp,
        mek_generation: 0,
    }
}

#[test]
fn empty_frame_rejected() {
    let err = fragment_frame(
        test_shape([0u8; STREAM_ID_LEN], 1, true, Codec::Vp9, 0),
        &[],
    )
    .unwrap_err();
    assert_eq!(err, FragmentError::EmptyFrame);
}

#[test]
fn exactly_max_fragments_succeeds() {
    // 255 full chunks — the u8 wire ceiling, must round-trip.
    let frame = vec![0u8; FRAGMENT_PAYLOAD_LIMIT * MAX_FRAGMENTS_PER_FRAME];
    let frags = fragment_frame(
        test_shape([2u8; STREAM_ID_LEN], 1, true, Codec::Vp9, 0),
        &frame,
    )
    .unwrap();
    assert_eq!(frags.len(), 255);
    assert_eq!(frags[254].frag_total, 255);
}

#[test]
fn one_over_max_fragments_errors_instead_of_panicking() {
    // Regression: 256 chunks passed the old `> 256` bound check and
    // panicked at the u8 conversion.
    let frame = vec![0u8; FRAGMENT_PAYLOAD_LIMIT * MAX_FRAGMENTS_PER_FRAME + 1];
    let err = fragment_frame(
        test_shape([2u8; STREAM_ID_LEN], 1, true, Codec::Vp9, 0),
        &frame,
    )
    .unwrap_err();
    assert_eq!(err, FragmentError::TooManyFragments(256));
}

#[test]
fn small_frame_one_fragment() {
    let frame = vec![0xABu8; 1024];
    let frags = fragment_frame(
        test_shape([1u8; STREAM_ID_LEN], 7, true, Codec::Vp9, 100),
        &frame,
    )
    .unwrap();
    assert_eq!(frags.len(), 1);
    assert_eq!(frags[0].frame_seq, 7);
    assert!(frags[0].keyframe);
    assert_eq!(frags[0].frag_total, 1);
    assert_eq!(frags[0].payload, frame);
}

#[test]
fn large_frame_multiple_fragments() {
    let frame = vec![0x55u8; FRAGMENT_PAYLOAD_LIMIT * 3 + 100];
    let frags = fragment_frame(
        test_shape([2u8; STREAM_ID_LEN], 12, false, Codec::Vp9, 200),
        &frame,
    )
    .unwrap();
    assert_eq!(frags.len(), 4);
    assert_eq!(frags[3].frag_index, 3);
    assert_eq!(frags[3].frag_total, 4);
    // Concatenating the payloads must reproduce the original bytes.
    let reassembled: Vec<u8> = frags.iter().flat_map(|f| f.payload.clone()).collect();
    assert_eq!(reassembled, frame);
}

#[test]
fn fec_encode_then_reconstruct_when_no_drops() {
    let frame = vec![0xAAu8; FRAGMENT_PAYLOAD_LIMIT * 3 + 100];
    let frags = fragment_frame_with_fec(
        test_shape([9u8; STREAM_ID_LEN], 42, true, Codec::Vp9, 500),
        &frame,
        2,
    )
    .unwrap();
    assert_eq!(frags.data.len(), 4);
    assert_eq!(frags.parity.len(), 2);

    let received_data: Vec<(u8, Vec<u8>)> = frags
        .data
        .iter()
        .map(|f| (f.frag_index, f.payload.clone()))
        .collect();
    let received_parity: Vec<(u8, Vec<u8>)> = frags
        .parity
        .iter()
        .map(|f| (f.parity_index, f.payload.clone()))
        .collect();
    let recovered = reconstruct_frame(
        &received_data,
        &received_parity,
        4,
        2,
        u32::try_from(frame.len()).unwrap(),
    )
    .unwrap();
    assert_eq!(recovered, frame);
}

#[test]
fn fec_recovers_from_two_dropped_data_shards() {
    // 4 data shards + 2 parity. Drop 2 data shards. Reconstruct
    // from remaining 2 data + 2 parity = 4 total ≥ 4 needed.
    let frame: Vec<u8> = (0..FRAGMENT_PAYLOAD_LIMIT * 3 + 7)
        .map(|i| u8::try_from(i & 0xff).unwrap())
        .collect();
    let frags = fragment_frame_with_fec(
        test_shape([3u8; STREAM_ID_LEN], 7, true, Codec::Vp9, 100),
        &frame,
        2,
    )
    .unwrap();

    // Drop frag_index 1 and 3.
    let received_data: Vec<(u8, Vec<u8>)> = frags
        .data
        .iter()
        .filter(|f| f.frag_index != 1 && f.frag_index != 3)
        .map(|f| (f.frag_index, f.payload.clone()))
        .collect();
    let received_parity: Vec<(u8, Vec<u8>)> = frags
        .parity
        .iter()
        .map(|f| (f.parity_index, f.payload.clone()))
        .collect();
    let recovered = reconstruct_frame(
        &received_data,
        &received_parity,
        4,
        2,
        u32::try_from(frame.len()).unwrap(),
    )
    .unwrap();
    assert_eq!(recovered, frame);
}

#[test]
fn fec_data_fragments_carry_unpadded_bytes() {
    // Systematic-code invariant (RFC 6330/8627): data fragments
    // carry only real object bytes, never FEC padding. A frame whose
    // length is not a multiple of shard_size DOES pad the last shard
    // for parity computation — but that padding must not ride the
    // wire. The in-order concat of data payloads is the exact frame.
    let frame: Vec<u8> = (0..FRAGMENT_PAYLOAD_LIMIT * 2 + 33)
        .map(|i| u8::try_from(i & 0xff).unwrap())
        .collect();
    let frags = fragment_frame_with_fec(
        test_shape([1u8; STREAM_ID_LEN], 3, true, Codec::Vp9, 7),
        &frame,
        1,
    )
    .unwrap();
    let total: usize = frags.data.iter().map(|f| f.payload.len()).sum();
    assert_eq!(total, frame.len(), "data fragments must carry no padding");
    let mut concat = Vec::new();
    for f in &frags.data {
        concat.extend_from_slice(&f.payload);
    }
    assert_eq!(concat, frame);
}

#[test]
fn fec_reconstruct_repads_received_short_shard() {
    // The last data shard is short (real bytes). Receive ALL data
    // (incl. the short last) + parity, drop one FULL middle data
    // shard. reconstruct_frame must re-pad the received short shard
    // to shard_size so RS sees the exact shards the sender encoded.
    let frame: Vec<u8> = (0..FRAGMENT_PAYLOAD_LIMIT * 2 + 33)
        .map(|i| u8::try_from(i & 0xff).unwrap())
        .collect();
    let frags = fragment_frame_with_fec(
        test_shape([2u8; STREAM_ID_LEN], 4, true, Codec::Vp9, 9),
        &frame,
        1,
    )
    .unwrap();
    assert_eq!(frags.data.len(), 3);
    // Drop the FULL middle data shard (index 1); keep 0 and the
    // short last (index 2), plus parity.
    let received_data: Vec<(u8, Vec<u8>)> = frags
        .data
        .iter()
        .filter(|f| f.frag_index != 1)
        .map(|f| (f.frag_index, f.payload.clone()))
        .collect();
    let received_parity: Vec<(u8, Vec<u8>)> = frags
        .parity
        .iter()
        .map(|f| (f.parity_index, f.payload.clone()))
        .collect();
    let recovered = reconstruct_frame(
        &received_data,
        &received_parity,
        3,
        1,
        u32::try_from(frame.len()).unwrap(),
    )
    .unwrap();
    assert_eq!(recovered, frame);
}

#[test]
fn fec_fails_when_too_many_shards_dropped() {
    // 4 data + 2 parity. Drop 3 data + 1 parity. Only 2 shards
    // remain — below the 4-shard threshold for reconstruction.
    let frame = vec![0x77u8; FRAGMENT_PAYLOAD_LIMIT * 3 + 50];
    let frags = fragment_frame_with_fec(
        test_shape([4u8; STREAM_ID_LEN], 8, true, Codec::Vp9, 200),
        &frame,
        2,
    )
    .unwrap();
    let received_data: Vec<(u8, Vec<u8>)> = frags
        .data
        .iter()
        .filter(|f| f.frag_index == 0)
        .map(|f| (f.frag_index, f.payload.clone()))
        .collect();
    let received_parity: Vec<(u8, Vec<u8>)> = frags
        .parity
        .iter()
        .filter(|f| f.parity_index == 0)
        .map(|f| (f.parity_index, f.payload.clone()))
        .collect();
    let err = reconstruct_frame(
        &received_data,
        &received_parity,
        4,
        2,
        u32::try_from(frame.len()).unwrap(),
    )
    .unwrap_err();
    match err {
        FragmentError::Fec(_) => {}
        other => panic!("expected Fec error, got {other:?}"),
    }
}

#[test]
fn fec_zero_parity_rejected() {
    let frame = vec![0u8; 100];
    let err = fragment_frame_with_fec(
        test_shape([0u8; STREAM_ID_LEN], 1, true, Codec::Vp9, 0),
        &frame,
        0,
    )
    .unwrap_err();
    assert_eq!(err, FragmentError::ZeroParity);
}

#[test]
fn parity_signing_bytes_layout() {
    let p = VideoParityFragment {
        stream_id: [0xCDu8; STREAM_ID_LEN],
        frame_seq: 0x0102_0304,
        parity_index: 1,
        parity_total: 2,
        data_count: 4,
        codec: Codec::H264,
        frame_len: 1234,
        timestamp: 0xCAFE_BABE,
        mek_generation: 0,
        payload: vec![0x10, 0x20],
        signature: Vec::new(),
    };
    let bytes = parity_signing_bytes(&p);
    assert_eq!(&bytes[..STREAM_ID_LEN], &[0xCDu8; STREAM_ID_LEN]);
    assert_eq!(
        &bytes[STREAM_ID_LEN..STREAM_ID_LEN + 4],
        &0x0102_0304u32.to_le_bytes()
    );
    assert_eq!(bytes[STREAM_ID_LEN + 4], 1);
    assert_eq!(bytes[STREAM_ID_LEN + 5], 2);
    assert_eq!(bytes[STREAM_ID_LEN + 6], 4);
    assert_eq!(bytes[STREAM_ID_LEN + 7], Codec::H264.wire_byte());
    assert_eq!(
        &bytes[STREAM_ID_LEN + 8..STREAM_ID_LEN + 12],
        &1234u32.to_le_bytes()
    );
    assert_eq!(
        &bytes[STREAM_ID_LEN + 12..STREAM_ID_LEN + 16],
        &0xCAFE_BABEu32.to_le_bytes()
    );
    assert_eq!(
        &bytes[STREAM_ID_LEN + 16..STREAM_ID_LEN + 24],
        &0u64.to_le_bytes()
    );
    assert_eq!(&bytes[STREAM_ID_LEN + 24..], &[0x10, 0x20]);
}

#[test]
fn signing_bytes_match_spec_layout() {
    // Architecture §10.6 line 2071 extended with the codec tag and
    // the MEK generation — the canonical bytes-to-sign are
    // `(stream_id || frame_seq || frag_index || frag_total ||
    // keyframe || codec || timestamp || mek_generation || payload)`.
    let frag = VideoFragment {
        stream_id: [0xABu8; STREAM_ID_LEN],
        frame_seq: 0x1122_3344,
        frag_index: 2,
        frag_total: 5,
        keyframe: true,
        codec: Codec::Vp8,
        timestamp: 0xDEAD_BEEF,
        mek_generation: 0,
        payload: vec![0x01, 0x02, 0x03],
        signature: Vec::new(),
    };
    let bytes = fragment_signing_bytes(&frag);
    assert_eq!(&bytes[..STREAM_ID_LEN], &[0xABu8; STREAM_ID_LEN]);
    assert_eq!(
        &bytes[STREAM_ID_LEN..STREAM_ID_LEN + 4],
        &0x1122_3344u32.to_le_bytes()
    );
    assert_eq!(bytes[STREAM_ID_LEN + 4], 2);
    assert_eq!(bytes[STREAM_ID_LEN + 5], 5);
    assert_eq!(bytes[STREAM_ID_LEN + 6], 1);
    assert_eq!(bytes[STREAM_ID_LEN + 7], Codec::Vp8.wire_byte());
    assert_eq!(
        &bytes[STREAM_ID_LEN + 8..STREAM_ID_LEN + 12],
        &0xDEAD_BEEFu32.to_le_bytes()
    );
    assert_eq!(
        &bytes[STREAM_ID_LEN + 12..STREAM_ID_LEN + 20],
        &0u64.to_le_bytes()
    );
    assert_eq!(&bytes[STREAM_ID_LEN + 20..], &[0x01, 0x02, 0x03]);
}

#[test]
fn codec_byte_is_signature_covered() {
    // Codec-confusion defense: flipping ONLY the codec tag must
    // change the signing bytes, so a re-labeled fragment fails
    // signature verification at every honest receiver.
    let mut frag = VideoFragment {
        stream_id: [7u8; STREAM_ID_LEN],
        frame_seq: 1,
        frag_index: 0,
        frag_total: 1,
        keyframe: true,
        codec: Codec::Vp9,
        timestamp: 0,
        mek_generation: 0,
        payload: vec![0xFF],
        signature: Vec::new(),
    };
    let vp9_bytes = fragment_signing_bytes(&frag);
    frag.codec = Codec::H264;
    assert_ne!(vp9_bytes, fragment_signing_bytes(&frag));
}

#[test]
fn rejects_frame_exceeding_max_fragments() {
    let frame = vec![0u8; FRAGMENT_PAYLOAD_LIMIT * (MAX_FRAGMENTS_PER_FRAME + 1)];
    let err = fragment_frame(
        test_shape([3u8; STREAM_ID_LEN], 1, true, Codec::Vp9, 0),
        &frame,
    )
    .unwrap_err();
    match err {
        FragmentError::TooManyFragments(n) => {
            assert!(n > MAX_FRAGMENTS_PER_FRAME);
        }
        _ => panic!("wrong error variant"),
    }
}
