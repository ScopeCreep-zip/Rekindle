use rekindle_transport_ipc::v3::audit::chain::{AuditChain, LinkInput};
use rekindle_transport_ipc::v3::session::rotation::compute_rotation_link;
use rekindle_transport_ipc::v3::crypto::keys::derive_rotation_keys;

fn make_input(seq: u64) -> LinkInput {
    LinkInput {
        session_seq: seq,
        envelope_hash: [seq as u8; 32],
        header_hash: [(seq + 1) as u8; 32],
        ciphertext_hash: [(seq + 2) as u8; 32],
    }
}

#[test]
fn rotation_link_incorporates_pre_terminal() {
    let link_a = compute_rotation_link(
        &[0xCC; 32], // combined_secret
        &[0x11; 32], // pre_terminal_link A
        &uuid::Uuid::nil(),
        1, 1,
    );
    let link_b = compute_rotation_link(
        &[0xCC; 32],
        &[0x22; 32], // pre_terminal_link B — different
        &uuid::Uuid::nil(),
        1, 1,
    );
    assert_ne!(link_a, link_b);
}

#[test]
fn rotation_link_incorporates_rotation_id() {
    let id_a = uuid::Uuid::from_u128(1);
    let id_b = uuid::Uuid::from_u128(2);
    let link_a = compute_rotation_link(&[0xCC; 32], &[0x11; 32], &id_a, 1, 1);
    let link_b = compute_rotation_link(&[0xCC; 32], &[0x11; 32], &id_b, 1, 1);
    assert_ne!(link_a, link_b);
}

#[test]
fn rotation_link_incorporates_generations() {
    let link_a = compute_rotation_link(&[0xCC; 32], &[0x11; 32], &uuid::Uuid::nil(), 1, 1);
    let link_b = compute_rotation_link(&[0xCC; 32], &[0x11; 32], &uuid::Uuid::nil(), 2, 1);
    let link_c = compute_rotation_link(&[0xCC; 32], &[0x11; 32], &uuid::Uuid::nil(), 1, 2);
    assert_ne!(link_a, link_b);
    assert_ne!(link_a, link_c);
    assert_ne!(link_b, link_c);
}

#[test]
fn rotation_link_is_deterministic() {
    let a = compute_rotation_link(&[0xCC; 32], &[0x11; 32], &uuid::Uuid::nil(), 5, 5);
    let b = compute_rotation_link(&[0xCC; 32], &[0x11; 32], &uuid::Uuid::nil(), 5, 5);
    assert_eq!(a, b);
}

#[test]
fn chain_continues_from_rotation_link() {
    let old_key = [0xAA; 32];
    let mut pre_chain = AuditChain::new(old_key, [0xBB; 32]);
    for seq in 0..10 {
        pre_chain.advance(make_input(seq));
    }
    let pre_terminal = pre_chain.current_link();

    let combined_secret = [0xCC; 32];
    let rotation_id = uuid::Uuid::from_u128(42);
    let rot_link = compute_rotation_link(&combined_secret, &pre_terminal, &rotation_id, 1, 1);

    let new_keys = derive_rotation_keys(&combined_secret);
    let mut post_chain = AuditChain::with_rotation_anchor(new_keys.audit_d2l, rot_link, 10);

    for seq in 10..20 {
        post_chain.advance(make_input(seq));
    }

    assert_ne!(post_chain.current_link(), [0u8; 32]);
    assert_ne!(post_chain.current_link(), pre_terminal);
    assert_eq!(post_chain.anchor_record().value, rot_link);
}

#[test]
fn post_rotation_chain_uses_new_audit_key() {
    let old_key = [0xAA; 32];
    let new_key = [0xDD; 32];
    let anchor = [0xEE; 32];

    let mut chain_old = AuditChain::new(old_key, anchor);
    let mut chain_new = AuditChain::new(new_key, anchor);

    let input = make_input(0);
    let link_old = chain_old.advance(input);
    let link_new = chain_new.advance(input);

    assert_ne!(link_old, link_new, "different audit keys must produce different links");
}
