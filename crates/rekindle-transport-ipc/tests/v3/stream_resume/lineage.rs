use rekindle_transport_ipc::v3::stream::resume::{
    TransferLineage, SessionAuditFragment, LineageVerifyError,
};

fn id(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}

fn make_fragment(
    session_id: uuid::Uuid,
    anchor: [u8; 32],
    start_seq: u64,
    end_seq: u64,
    end_link: [u8; 32],
) -> SessionAuditFragment {
    SessionAuditFragment {
        session_id,
        anchor,
        start_session_seq: start_seq,
        end_session_seq: end_seq,
        end_chain_link: end_link,
    }
}

#[test]
fn new_lineage_has_one_fragment() {
    let transfer_id = id(1);
    let frag = make_fragment(id(100), [0xAA; 32], 0, 99, [0x11; 32]);
    let lineage = TransferLineage::new(transfer_id, frag);
    assert_eq!(lineage.fragment_count(), 1);
    assert_eq!(lineage.transfer_id(), transfer_id);
}

#[test]
fn add_fragment_increments_count() {
    let transfer_id = id(1);
    let mut lineage = TransferLineage::new(
        transfer_id,
        make_fragment(id(100), [0xAA; 32], 0, 99, [0x11; 32]),
    );
    lineage.add_fragment(make_fragment(id(200), [0x11; 32], 0, 49, [0x22; 32]));
    assert_eq!(lineage.fragment_count(), 2);
}

#[test]
fn fragments_ordered_by_addition() {
    let transfer_id = id(1);
    let f1 = make_fragment(id(100), [0xAA; 32], 0, 99, [0x11; 32]);
    let f2 = make_fragment(id(200), [0x11; 32], 0, 49, [0x22; 32]);
    let f3 = make_fragment(id(300), [0x22; 32], 0, 29, [0x33; 32]);

    let mut lineage = TransferLineage::new(transfer_id, f1.clone());
    lineage.add_fragment(f2.clone());
    lineage.add_fragment(f3.clone());

    let frags = lineage.fragments();
    assert_eq!(frags[0].session_id, id(100));
    assert_eq!(frags[1].session_id, id(200));
    assert_eq!(frags[2].session_id, id(300));
}

#[test]
fn fragment_captures_session_id() {
    let frag = make_fragment(id(42), [0xAA; 32], 0, 99, [0x11; 32]);
    assert_eq!(frag.session_id, id(42));
}

#[test]
fn fragment_captures_anchor() {
    let anchor = [0xDD; 32];
    let frag = make_fragment(id(1), anchor, 0, 99, [0x11; 32]);
    assert_eq!(frag.anchor, anchor);
}

#[test]
fn fragment_captures_seq_range() {
    let frag = make_fragment(id(1), [0xAA; 32], 50, 150, [0x11; 32]);
    assert_eq!(frag.start_session_seq, 50);
    assert_eq!(frag.end_session_seq, 150);
}

#[test]
fn fragment_captures_end_chain_link() {
    let end_link = [0xEE; 32];
    let frag = make_fragment(id(1), [0xAA; 32], 0, 99, end_link);
    assert_eq!(frag.end_chain_link, end_link);
}

#[test]
fn verify_lineage_contiguous_succeeds() {
    let transfer_id = id(1);
    // Fragment 1 ends with link [0x11], fragment 2 anchors on [0x11], ends with [0x22],
    // fragment 3 anchors on [0x22]
    let f1 = make_fragment(id(100), [0xAA; 32], 0, 99, [0x11; 32]);
    let f2 = make_fragment(id(200), [0x11; 32], 0, 49, [0x22; 32]);
    let f3 = make_fragment(id(300), [0x22; 32], 0, 29, [0x33; 32]);

    let mut lineage = TransferLineage::new(transfer_id, f1);
    lineage.add_fragment(f2);
    lineage.add_fragment(f3);

    let result = lineage.verify_continuity();
    assert!(result.is_ok(), "Contiguous lineage must verify: {result:?}");
}

#[test]
fn verify_lineage_gap_detected() {
    let transfer_id = id(1);
    let f1 = make_fragment(id(100), [0xAA; 32], 0, 99, [0x11; 32]);
    // Fragment 2 anchors on [0xFF] — doesn't match fragment 1's end_chain_link [0x11]
    let f2 = make_fragment(id(200), [0xFF; 32], 0, 49, [0x22; 32]);

    let mut lineage = TransferLineage::new(transfer_id, f1);
    lineage.add_fragment(f2);

    let result = lineage.verify_continuity();
    assert!(result.is_err());
    match result.unwrap_err() {
        LineageVerifyError::ContinuityBreak { fragment_index } => {
            assert_eq!(fragment_index, 1, "Break must be at fragment 1");
        }
    }
}

#[test]
fn verify_lineage_single_fragment_succeeds() {
    let transfer_id = id(1);
    let lineage = TransferLineage::new(
        transfer_id,
        make_fragment(id(100), [0xAA; 32], 0, 99, [0x11; 32]),
    );
    assert!(lineage.verify_continuity().is_ok());
}

#[test]
fn verify_lineage_empty_after_construction_impossible() {
    // TransferLineage::new requires one fragment, so empty is not constructable.
    // This test verifies that the API prevents empty lineage at the type level.
    let transfer_id = id(1);
    let lineage = TransferLineage::new(
        transfer_id,
        make_fragment(id(100), [0xAA; 32], 0, 0, [0x11; 32]),
    );
    assert_eq!(lineage.fragment_count(), 1);
}

#[test]
fn lineage_transfer_id_is_consistent() {
    let transfer_id = id(42);
    let mut lineage = TransferLineage::new(
        transfer_id,
        make_fragment(id(100), [0xAA; 32], 0, 99, [0x11; 32]),
    );
    lineage.add_fragment(make_fragment(id(200), [0x11; 32], 0, 49, [0x22; 32]));
    assert_eq!(lineage.transfer_id(), transfer_id);
}

#[test]
fn total_bytes_across_fragments() {
    let transfer_id = id(1);
    let f1 = make_fragment(id(100), [0xAA; 32], 0, 99, [0x11; 32]);  // 100 seqs
    let f2 = make_fragment(id(200), [0x11; 32], 0, 49, [0x22; 32]);  // 50 seqs
    let f3 = make_fragment(id(300), [0x22; 32], 0, 29, [0x33; 32]);  // 30 seqs

    let mut lineage = TransferLineage::new(transfer_id, f1);
    lineage.add_fragment(f2);
    lineage.add_fragment(f3);

    // Total session_seq span across all fragments
    let total: u64 = lineage.fragments().iter()
        .map(|f| f.end_session_seq - f.start_session_seq + 1)
        .sum();
    assert_eq!(total, 100 + 50 + 30);
}
