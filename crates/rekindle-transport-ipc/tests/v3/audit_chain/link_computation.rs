use rekindle_transport_ipc::v3::audit::chain::{AuditChain, LinkInput};

fn test_audit_key() -> [u8; 32] {
    [0xAA; 32]
}

fn test_anchor() -> [u8; 32] {
    [0xBB; 32]
}

fn make_input(session_seq: u64) -> LinkInput {
    LinkInput {
        session_seq,
        envelope_hash: [session_seq as u8; 32],
        header_hash: [(session_seq + 1) as u8; 32],
        ciphertext_hash: [(session_seq + 2) as u8; 32],
    }
}

fn make_input_no_header(session_seq: u64) -> LinkInput {
    LinkInput {
        session_seq,
        envelope_hash: [session_seq as u8; 32],
        header_hash: [0u8; 32],
        ciphertext_hash: [(session_seq + 2) as u8; 32],
    }
}

#[test]
fn first_link_incorporates_anchor() {
    let mut chain = AuditChain::new(test_audit_key(), test_anchor());
    let link = chain.advance(make_input(0));
    assert_ne!(link, [0u8; 32], "link must not be zero");
    assert_ne!(link, test_anchor(), "link must differ from anchor");
    assert_eq!(link.len(), 32);
}

#[test]
fn subsequent_link_chains_previous() {
    let mut chain = AuditChain::new(test_audit_key(), test_anchor());
    let link0 = chain.advance(make_input(0));
    let link1 = chain.advance(make_input(1));
    assert_ne!(link0, link1);
}

#[test]
fn link_is_32_bytes() {
    let mut chain = AuditChain::new(test_audit_key(), test_anchor());
    let link = chain.advance(make_input(0));
    assert_eq!(link.len(), 32);
}

#[test]
fn link_is_deterministic() {
    let mut chain_a = AuditChain::new(test_audit_key(), test_anchor());
    let mut chain_b = AuditChain::new(test_audit_key(), test_anchor());
    let link_a = chain_a.advance(make_input(0));
    let link_b = chain_b.advance(make_input(0));
    assert_eq!(link_a, link_b);
}

#[test]
fn different_session_seq_produces_different_link() {
    let mut chain_a = AuditChain::new(test_audit_key(), test_anchor());
    let mut chain_b = AuditChain::new(test_audit_key(), test_anchor());
    let input_a = LinkInput {
        session_seq: 0,
        envelope_hash: [0x11; 32],
        header_hash: [0x22; 32],
        ciphertext_hash: [0x33; 32],
    };
    let mut input_b = input_a;
    input_b.session_seq = 1;
    let link_a = chain_a.advance(input_a);
    let link_b = chain_b.advance(input_b);
    assert_ne!(link_a, link_b);
}

#[test]
fn different_ciphertext_produces_different_link() {
    let mut chain_a = AuditChain::new(test_audit_key(), test_anchor());
    let mut chain_b = AuditChain::new(test_audit_key(), test_anchor());
    let input_a = LinkInput {
        session_seq: 0,
        envelope_hash: [0x11; 32],
        header_hash: [0x22; 32],
        ciphertext_hash: [0x33; 32],
    };
    let mut input_b = input_a;
    input_b.ciphertext_hash = [0x44; 32];
    let link_a = chain_a.advance(input_a);
    let link_b = chain_b.advance(input_b);
    assert_ne!(link_a, link_b);
}

#[test]
fn different_audit_key_produces_different_link() {
    let mut chain_a = AuditChain::new([0xAA; 32], test_anchor());
    let mut chain_b = AuditChain::new([0xCC; 32], test_anchor());
    let input = make_input(0);
    let link_a = chain_a.advance(input);
    let link_b = chain_b.advance(input);
    assert_ne!(link_a, link_b);
}

#[test]
fn non_data_lane_uses_zero_header_hash() {
    let mut chain = AuditChain::new(test_audit_key(), test_anchor());
    let link = chain.advance(make_input_no_header(0));
    assert_ne!(link, [0u8; 32]);
    // Verify it differs from a Data Lane input with a non-zero header
    let mut chain2 = AuditChain::new(test_audit_key(), test_anchor());
    let link2 = chain2.advance(make_input(0));
    assert_ne!(link, link2, "zero header_hash must produce different link than non-zero");
}

#[test]
fn current_link_queryable() {
    let mut chain = AuditChain::new(test_audit_key(), test_anchor());
    assert_eq!(chain.current_link(), test_anchor());
    let link = chain.advance(make_input(0));
    assert_eq!(chain.current_link(), link);
}

#[test]
fn chain_length_increments() {
    let mut chain = AuditChain::new(test_audit_key(), test_anchor());
    assert_eq!(chain.length(), 0);
    chain.advance(make_input(0));
    assert_eq!(chain.length(), 1);
    chain.advance(make_input(1));
    assert_eq!(chain.length(), 2);
}
