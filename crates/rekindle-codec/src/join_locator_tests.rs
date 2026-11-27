use super::*;
use rekindle_types::{governance::OpHash, id::PseudonymKey};

#[test]
fn the_sealed_locator_opens_only_for_its_inviter_and_invite() {
    let inviter = ed25519_dalek::SigningKey::from_bytes(&[1; 32]);
    let other = ed25519_dalek::SigningKey::from_bytes(&[2; 32]);
    let invite = [3; 16];
    let locator = JoinLocator {
        joined_op: OpRef {
            op_hash: OpHash([4; 32]),
            author: PseudonymKey([5; 32]),
            author_record: format!("VLD0:{}:{}", "A".repeat(43), "B".repeat(43)),
            author_seq: 0,
            device_key: [6; 32],
        },
    };
    let bytes = locator
        .seal(&inviter.verifying_key().to_bytes(), &invite)
        .unwrap();
    assert!(bytes.len() < MAX_ENTRY_LEN);
    assert_eq!(
        JoinLocator::open(&bytes, &inviter, &invite).unwrap(),
        locator
    );
    assert!(JoinLocator::open(&bytes, &other, &invite).is_err());
    assert!(JoinLocator::open(&bytes, &inviter, &[7; 16]).is_err());
    assert!(JoinLocator::open(&bytes[..HEADER_LEN], &inviter, &invite).is_err());
    assert!(JoinLocator::open(&[0; MAX_ENTRY_LEN + 1], &inviter, &invite).is_err());
}
