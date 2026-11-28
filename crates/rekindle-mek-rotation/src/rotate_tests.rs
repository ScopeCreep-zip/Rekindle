use super::*;

fn key(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn recipient(b: u8) -> RotationRecipient {
    RotationRecipient {
        pseudonym_hex: pseudonym_hex(&key(b)),
        route_blob: Vec::new(),
    }
}

#[test]
fn the_departed_member_is_never_a_rotation_candidate_even_as_us() {
    let departed = key(2);
    let candidates = rotation_candidates(
        &departed,
        &[recipient(1), recipient(3)],
        Some(key(2)),
        |_| true,
    );
    assert_eq!(candidates, vec![key(1), key(3)]);
}

#[test]
fn we_join_the_candidates_when_the_governance_lets_us_rotate() {
    let candidates = rotation_candidates(&key(9), &[recipient(1)], Some(key(4)), |c| *c != key(1));
    assert_eq!(candidates, vec![key(4)]);
}
