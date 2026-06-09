use rekindle_transport_ipc::v3::audit::chain::{AuditChain, LinkInput, verify_chain};

fn test_audit_key() -> [u8; 32] { [0xAA; 32] }
fn test_anchor() -> [u8; 32] { [0xBB; 32] }

fn make_input(seq: u64) -> LinkInput {
    LinkInput {
        session_seq: seq,
        envelope_hash: [seq as u8; 32],
        header_hash: [(seq + 1) as u8; 32],
        ciphertext_hash: [(seq + 2) as u8; 32],
    }
}

fn build_chain(count: u64) -> (AuditChain, Vec<(u64, LinkInput, [u8; 32])>) {
    let mut chain = AuditChain::new(test_audit_key(), test_anchor());
    let mut entries = Vec::new();
    for seq in 0..count {
        let input = make_input(seq);
        let link = chain.advance(input);
        entries.push((seq, input, link));
    }
    (chain, entries)
}

#[test]
fn chain_of_10_frames_verifiable() {
    let (chain, entries) = build_chain(10);
    let inputs: Vec<LinkInput> = entries.iter().map(|(_, input, _)| *input).collect();
    let expected_links: Vec<[u8; 32]> = entries.iter().map(|(_, _, link)| *link).collect();
    let result = verify_chain(&test_audit_key(), &test_anchor(), &inputs, &expected_links);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), chain.current_link());
}

#[test]
fn tampered_frame_breaks_chain() {
    let (_chain, entries) = build_chain(10);
    let mut inputs: Vec<LinkInput> = entries.iter().map(|(_, input, _)| *input).collect();
    let expected_links: Vec<[u8; 32]> = entries.iter().map(|(_, _, link)| *link).collect();
    inputs[5].ciphertext_hash = [0xFF; 32];
    let result = verify_chain(&test_audit_key(), &test_anchor(), &inputs, &expected_links);
    assert!(result.is_err());
    let err = format!("{:?}", result.unwrap_err());
    assert!(err.contains("5"), "Error must identify the tampered position, got: {err}");
}

#[test]
fn deleted_frame_breaks_chain() {
    let (_chain, entries) = build_chain(10);
    let mut inputs: Vec<LinkInput> = entries.iter().map(|(_, input, _)| *input).collect();
    let expected_links: Vec<[u8; 32]> = entries.iter().map(|(_, _, link)| *link).collect();
    inputs.remove(3);
    // Length mismatch: 9 inputs vs 10 expected links
    let result = verify_chain(&test_audit_key(), &test_anchor(), &inputs, &expected_links);
    assert!(result.is_err());
}

#[test]
fn inserted_frame_breaks_chain() {
    let (chain, entries) = build_chain(10);
    let mut inputs: Vec<LinkInput> = entries.iter().map(|(_, input, _)| *input).collect();
    let expected_links: Vec<[u8; 32]> = entries.iter().map(|(_, _, link)| *link).collect();
    let original_final_link = chain.current_link();
    let extra = LinkInput {
        session_seq: 99,
        envelope_hash: [0xDE; 32],
        header_hash: [0xAD; 32],
        ciphertext_hash: [0xBE; 32],
    };
    inputs.insert(5, extra);
    // Length mismatch: 11 inputs vs 10 expected links
    let result = verify_chain(&test_audit_key(), &test_anchor(), &inputs, &expected_links);
    assert!(result.is_err());
    // Verify the original chain is unaffected
    assert_eq!(chain.current_link(), original_final_link);
}

#[test]
fn reordered_frames_break_chain() {
    let (_chain, entries) = build_chain(10);
    let mut inputs: Vec<LinkInput> = entries.iter().map(|(_, input, _)| *input).collect();
    let expected_links: Vec<[u8; 32]> = entries.iter().map(|(_, _, link)| *link).collect();
    inputs.swap(6, 7);
    let result = verify_chain(&test_audit_key(), &test_anchor(), &inputs, &expected_links);
    assert!(result.is_err());
}

#[test]
fn chain_of_1000_frames_verifiable() {
    let (chain, entries) = build_chain(1000);
    let inputs: Vec<LinkInput> = entries.iter().map(|(_, input, _)| *input).collect();
    let expected_links: Vec<[u8; 32]> = entries.iter().map(|(_, _, link)| *link).collect();
    let result = verify_chain(&test_audit_key(), &test_anchor(), &inputs, &expected_links);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), chain.current_link());
}

#[test]
fn empty_chain_verifies_to_anchor() {
    let result = verify_chain(&test_audit_key(), &test_anchor(), &[], &[]);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), test_anchor());
}
