use rekindle_transport_ipc::v3::audit::chain::{AuditChain, AnchorSource};

#[test]
fn handshake_anchor_is_transcript_hash() {
    let transcript_hash = [0xCC; 32];
    let chain = AuditChain::new([0xAA; 32], transcript_hash);
    let record = chain.anchor_record();
    assert_eq!(record.value, transcript_hash);
    assert_eq!(record.source, AnchorSource::Handshake);
    assert_eq!(record.start_session_seq, 0);
}

#[test]
fn anchor_record_tracks_start_seq() {
    let chain = AuditChain::new([0xAA; 32], [0xBB; 32]);
    assert_eq!(chain.anchor_record().start_session_seq, 0);
}

#[test]
fn anchor_record_source_is_handshake() {
    let chain = AuditChain::new([0xAA; 32], [0xBB; 32]);
    assert_eq!(chain.anchor_record().source, AnchorSource::Handshake);
}

#[test]
fn rotation_anchor_differs_from_handshake() {
    let handshake_anchor = [0xBB; 32];
    let rotation_link = [0xDD; 32];
    let chain = AuditChain::with_rotation_anchor([0xAA; 32], rotation_link, 100);
    let record = chain.anchor_record();
    assert_eq!(record.value, rotation_link);
    assert_ne!(record.value, handshake_anchor);
    assert_eq!(record.source, AnchorSource::Rotation);
    assert_eq!(record.start_session_seq, 100);
}

#[test]
fn anchor_record_is_queryable_after_advance() {
    let mut chain = AuditChain::new([0xAA; 32], [0xBB; 32]);
    chain.advance(rekindle_transport_ipc::v3::audit::chain::LinkInput {
        session_seq: 0,
        envelope_hash: [0x11; 32],
        header_hash: [0x22; 32],
        ciphertext_hash: [0x33; 32],
    });
    // Anchor doesn't change after advance
    assert_eq!(chain.anchor_record().value, [0xBB; 32]);
}
