use rekindle_transport_ipc::v3::audit::chain::{AuditChain, LinkInput, verify_chain};
use rekindle_transport_ipc::v3::audit::checkpoint::CheckpointData;

fn test_key() -> [u8; 32] { [0xAA; 32] }
fn test_anchor() -> [u8; 32] { [0xBB; 32] }

fn make_input(seq: u64) -> LinkInput {
    LinkInput {
        session_seq: seq,
        envelope_hash: [seq as u8; 32],
        header_hash: [(seq + 1) as u8; 32],
        ciphertext_hash: [(seq + 2) as u8; 32],
    }
}

fn build_chain_and_checkpoint(count: u64) -> (AuditChain, CheckpointData) {
    let mut chain = AuditChain::new(test_key(), test_anchor());
    for seq in 0..count {
        chain.advance(make_input(seq));
    }
    let cp = CheckpointData {
        chain_index: count.saturating_sub(1),
        chain_length: count,
        chain_link: chain.current_link(),
        anchor_link: test_anchor(),
        checkpoint_seq: 0,
    };
    (chain, cp)
}

#[test]
fn valid_checkpoint_accepted() {
    let (chain, cp) = build_chain_and_checkpoint(100);

    // Verify by rebuilding a second chain and comparing
    let mut verifier = AuditChain::new(test_key(), test_anchor());
    for seq in 0..100 {
        verifier.advance(make_input(seq));
    }
    assert_eq!(verifier.current_link(), cp.chain_link);
    assert_eq!(verifier.length(), cp.chain_length);
    assert_eq!(verifier.current_link(), chain.current_link());

    // Verify via verify_chain which recomputes from anchor through all inputs
    let inputs: Vec<LinkInput> = (0..100).map(make_input).collect();
    let expected_links: Vec<[u8; 32]> = chain.links().to_vec();
    let final_link = verify_chain(&test_key(), &test_anchor(), &inputs, &expected_links)
        .expect("verify_chain must succeed on valid inputs");
    assert_eq!(final_link, cp.chain_link);
}

#[test]
fn chain_link_mismatch_detected() {
    let (_chain, mut cp) = build_chain_and_checkpoint(100);
    cp.chain_link = [0xFF; 32];
    let mut verifier = AuditChain::new(test_key(), test_anchor());
    for seq in 0..100 {
        verifier.advance(make_input(seq));
    }
    assert_ne!(verifier.current_link(), cp.chain_link);
}

#[test]
fn chain_length_mismatch_detected() {
    let (_chain, cp) = build_chain_and_checkpoint(100);
    let mut verifier = AuditChain::new(test_key(), test_anchor());
    for seq in 0..99 {
        verifier.advance(make_input(seq));
    }
    assert_ne!(verifier.length(), cp.chain_length);
}

#[test]
fn anchor_mismatch_detected() {
    let (_chain, cp) = build_chain_and_checkpoint(10);
    let wrong_anchor = [0xCC; 32];
    let mut verifier = AuditChain::new(test_key(), wrong_anchor);
    for seq in 0..10 {
        verifier.advance(make_input(seq));
    }
    assert_ne!(verifier.current_link(), cp.chain_link, "wrong anchor must produce different chain");
}

#[test]
fn checkpoint_data_fields_populated() {
    let (_chain, cp) = build_chain_and_checkpoint(50);
    assert_eq!(cp.chain_index, 49);
    assert_eq!(cp.chain_length, 50);
    assert_eq!(cp.anchor_link, test_anchor());
    assert_ne!(cp.chain_link, [0u8; 32]);
}
