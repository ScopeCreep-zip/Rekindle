use super::*;

const PEER: &str = "peer_pubkey_hex";
const SID: [u8; 16] = [0xaa; 16];

/// Test helper: build a `DmVideoFragment` from the positional args
/// the suite used before the struct refactor, keeping each case terse.
fn frag(
    stream_id: [u8; 16],
    frame_seq: u32,
    fragment_index: u16,
    fragment_count: u16,
    keyframe: bool,
    timestamp: u32,
    chunk: Vec<u8>,
) -> DmVideoFragment {
    DmVideoFragment {
        stream_id,
        frame_seq,
        fragment_index,
        fragment_count,
        keyframe,
        codec: "vp9".to_string(),
        timestamp,
        chunk,
    }
}

#[test]
fn single_fragment_completes_frame() {
    let state = DmVideoReassemblyState::new();
    let frame = state.record_fragment(PEER, frag(SID, 1, 0, 1, true, 100, vec![1, 2, 3]));
    let assembled = frame.expect("single-fragment frame completes on first call");
    assert_eq!(assembled.frame_seq, 1);
    assert!(assembled.keyframe);
    assert_eq!(assembled.timestamp, 100);
    assert_eq!(assembled.data, vec![1, 2, 3]);
}

#[test]
fn multi_fragment_assembles_in_order() {
    let state = DmVideoReassemblyState::new();
    assert!(state
        .record_fragment(PEER, frag(SID, 2, 0, 3, false, 200, vec![1, 2]))
        .is_none());
    assert!(state
        .record_fragment(PEER, frag(SID, 2, 1, 3, false, 200, vec![3, 4]))
        .is_none());
    let assembled = state
        .record_fragment(PEER, frag(SID, 2, 2, 3, false, 200, vec![5, 6]))
        .expect("final fragment completes frame");
    assert_eq!(assembled.data, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn multi_fragment_out_of_order_still_assembles() {
    let state = DmVideoReassemblyState::new();
    assert!(state
        .record_fragment(PEER, frag(SID, 3, 2, 3, false, 300, vec![5, 6]))
        .is_none());
    assert!(state
        .record_fragment(PEER, frag(SID, 3, 0, 3, false, 300, vec![1, 2]))
        .is_none());
    let assembled = state
        .record_fragment(PEER, frag(SID, 3, 1, 3, false, 300, vec![3, 4]))
        .expect("frame completes regardless of arrival order");
    assert_eq!(assembled.data, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn invalid_fragment_index_returns_none() {
    let state = DmVideoReassemblyState::new();
    // fragment_index == fragment_count is out-of-range.
    assert!(state
        .record_fragment(PEER, frag(SID, 1, 2, 2, false, 0, vec![1]))
        .is_none());
    // fragment_count = 0 is malformed.
    assert!(state
        .record_fragment(PEER, frag(SID, 1, 0, 0, false, 0, vec![1]))
        .is_none());
}

#[test]
fn forget_peer_drops_all_state() {
    let state = DmVideoReassemblyState::new();
    // Partial frame in flight for PEER.
    assert!(state
        .record_fragment(PEER, frag(SID, 4, 0, 2, false, 400, vec![1, 2]))
        .is_none());
    state.forget_peer(PEER);
    // Next fragment looks like a fresh frame after forget — it
    // should NOT auto-complete because state was cleared.
    assert!(state
        .record_fragment(PEER, frag(SID, 4, 1, 2, false, 400, vec![3, 4]))
        .is_none());
}

#[test]
fn changing_fragment_count_mid_frame_resets() {
    let state = DmVideoReassemblyState::new();
    assert!(state
        .record_fragment(PEER, frag(SID, 5, 0, 3, false, 500, vec![1, 2]))
        .is_none());
    // Sender suddenly claims a different fragment_count for the
    // same frame — treat as a fresh frame.
    let result = state.record_fragment(PEER, frag(SID, 5, 0, 1, false, 500, vec![9, 9]));
    assert!(result.is_some()); // new fragment_count=1, fragment_index=0 → completes
    assert_eq!(result.unwrap().data, vec![9, 9]);
}

#[test]
fn changing_codec_mid_frame_resets() {
    let state = DmVideoReassemblyState::new();
    assert!(state
        .record_fragment(PEER, frag(SID, 6, 0, 2, false, 600, vec![1, 2]))
        .is_none());
    // Same frame, same fragment_count, different codec tag — the
    // vp9 chunk from above must NOT merge into this h264 frame.
    let second = DmVideoFragment {
        stream_id: SID,
        frame_seq: 6,
        fragment_index: 1,
        fragment_count: 2,
        keyframe: false,
        codec: "h264".to_string(),
        timestamp: 600,
        chunk: vec![3, 4],
    };
    assert!(
        state.record_fragment(PEER, second).is_none(),
        "codec switch mid-frame must reset, not complete the frame"
    );
    // The reset frame is h264 and holds only the resetting
    // fragment's chunk (index 1). Supplying index 0 completes it —
    // and the assembled data proves the vp9 chunk [1, 2] was
    // discarded, not merged.
    let assembled = state
        .record_fragment(
            PEER,
            DmVideoFragment {
                stream_id: SID,
                frame_seq: 6,
                fragment_index: 0,
                fragment_count: 2,
                keyframe: false,
                codec: "h264".to_string(),
                timestamp: 600,
                chunk: vec![9, 9],
            },
        )
        .expect("reset h264 frame completes from its two h264 chunks");
    assert_eq!(assembled.codec, "h264");
    assert_eq!(assembled.data, vec![9, 9, 3, 4]);
}

#[test]
fn different_streams_track_independently() {
    let state = DmVideoReassemblyState::new();
    let sid_a = [0xaa; 16];
    let sid_b = [0xbb; 16];
    assert!(state
        .record_fragment(PEER, frag(sid_a, 1, 0, 2, false, 600, vec![1, 2]))
        .is_none());
    // Different stream, same frame_seq — should NOT auto-complete A.
    let result = state.record_fragment(PEER, frag(sid_b, 1, 0, 1, false, 700, vec![9]));
    assert!(result.is_some());
    assert_eq!(result.unwrap().stream_id, sid_b);
    // A still pending, complete with the second fragment.
    let result_a = state.record_fragment(PEER, frag(sid_a, 1, 1, 2, false, 600, vec![3, 4]));
    assert!(result_a.is_some());
    assert_eq!(result_a.unwrap().stream_id, sid_a);
}

#[test]
fn fragment_payload_limit_matches_community_budget() {
    // Sanity check the published constant — frontend keyframe
    // splitter depends on this value, and the DM budget must stay
    // in lock-step with the community one (rekindle-video
    // fragment.rs, per-hop datagram rationale).
    assert_eq!(FRAGMENT_PAYLOAD_LIMIT, 4 * 1024);
}
