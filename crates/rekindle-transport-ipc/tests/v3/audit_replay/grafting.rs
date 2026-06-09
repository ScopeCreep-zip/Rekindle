use rekindle_transport_ipc::v3::audit::chain::{AuditChain, LinkInput};
use rekindle_transport_ipc::v3::audit::replay::graft_frames;

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

fn ideal_chain(count: u64) -> (AuditChain, Vec<LinkInput>) {
    let mut chain = AuditChain::new(test_key(), test_anchor());
    let mut inputs = Vec::new();
    for seq in 0..count {
        let input = make_input(seq);
        chain.advance(input);
        inputs.push(input);
    }
    (chain, inputs)
}

#[test]
fn graft_single_missing_frame() {
    let (ideal, all_inputs) = ideal_chain(10);
    let ideal_link = ideal.current_link();

    let mut gapped = AuditChain::new(test_key(), test_anchor());
    for seq in 0..5u64 {
        gapped.advance(all_inputs[seq as usize]);
    }

    graft_frames(&mut gapped, &[all_inputs[5]], &all_inputs[6..10]);
    assert_eq!(gapped.current_link(), ideal_link, "grafted chain must match ideal");
}

#[test]
fn graft_multiple_missing_frames() {
    let (ideal, all_inputs) = ideal_chain(10);
    let ideal_link = ideal.current_link();

    let mut gapped = AuditChain::new(test_key(), test_anchor());
    for seq in 0..3u64 {
        gapped.advance(all_inputs[seq as usize]);
    }

    graft_frames(&mut gapped, &all_inputs[3..6], &all_inputs[6..10]);
    assert_eq!(gapped.current_link(), ideal_link);
}

#[test]
fn graft_non_contiguous_missing() {
    let (ideal, all_inputs) = ideal_chain(10);
    let ideal_link = ideal.current_link();

    let mut gapped = AuditChain::new(test_key(), test_anchor());
    for seq in 0..4u64 {
        gapped.advance(all_inputs[seq as usize]);
    }

    graft_frames(
        &mut gapped,
        &[all_inputs[4], all_inputs[6]],
        &[all_inputs[5], all_inputs[7], all_inputs[8], all_inputs[9]],
    );
    assert_eq!(gapped.current_link(), ideal_link);
}

#[test]
fn graft_recomputes_downstream_links() {
    let (ideal, all_inputs) = ideal_chain(15);
    let ideal_link = ideal.current_link();

    let mut gapped = AuditChain::new(test_key(), test_anchor());
    for seq in 0..5u64 {
        gapped.advance(all_inputs[seq as usize]);
    }

    graft_frames(&mut gapped, &[all_inputs[5]], &all_inputs[6..15]);
    assert_eq!(gapped.current_link(), ideal_link,
        "all 9 downstream links must be recomputed correctly");
}

#[test]
fn tampered_linkinput_produces_divergent_chain() {
    let (ideal, all_inputs) = ideal_chain(10);

    let mut gapped = AuditChain::new(test_key(), test_anchor());
    for seq in 0..5u64 {
        gapped.advance(all_inputs[seq as usize]);
    }

    let mut tampered = all_inputs[5];
    tampered.ciphertext_hash = [0xFF; 32];

    graft_frames(&mut gapped, &[tampered], &all_inputs[6..10]);
    assert_ne!(
        gapped.current_link(),
        ideal.current_link(),
        "tampered LinkInput must produce divergent chain"
    );
}

#[test]
fn wrong_session_seq_produces_divergent_chain() {
    let (ideal, all_inputs) = ideal_chain(10);

    let mut gapped = AuditChain::new(test_key(), test_anchor());
    for seq in 0..5u64 {
        gapped.advance(all_inputs[seq as usize]);
    }

    let mut wrong_seq = all_inputs[5];
    wrong_seq.session_seq = 99;

    graft_frames(&mut gapped, &[wrong_seq], &all_inputs[6..10]);
    assert_ne!(
        gapped.current_link(),
        ideal.current_link(),
        "wrong session_seq must produce divergent chain"
    );
}

#[test]
fn chain_after_graft_matches_no_gap_chain() {
    for gap_pos in 0..10u64 {
        let (ideal, all_inputs) = ideal_chain(10);
        let ideal_link = ideal.current_link();

        let mut gapped = AuditChain::new(test_key(), test_anchor());
        for seq in 0..gap_pos {
            gapped.advance(all_inputs[seq as usize]);
        }
        let post_gap_start = (gap_pos + 1) as usize;

        graft_frames(
            &mut gapped,
            &[all_inputs[gap_pos as usize]],
            &all_inputs[post_gap_start..10],
        );
        assert_eq!(
            gapped.current_link(), ideal_link,
            "graft at pos {gap_pos} must produce identical chain"
        );
    }
}

#[test]
fn graft_empty_missing_set_is_noop() {
    let (ideal, all_inputs) = ideal_chain(5);
    let mut chain = AuditChain::new(test_key(), test_anchor());
    for input in &all_inputs {
        chain.advance(*input);
    }
    assert_eq!(chain.current_link(), ideal.current_link());

    graft_frames(&mut chain, &[], &[]);
    assert_eq!(chain.current_link(), ideal.current_link());
}
