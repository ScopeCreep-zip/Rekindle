use super::*;

fn hint(subkey: u32) -> HeadAdvanced {
    HeadAdvanced {
        author: [7; 32],
        member_record: format!("VLD0:{}:{}", "A".repeat(43), "b".repeat(43)),
        subkey,
        seq: u64::MAX,
    }
}

#[test]
fn a_hint_round_trips_under_its_relayer_and_fits_its_cap() {
    let relayer = SigningKey::from_bytes(&[3; 32]);
    let bytes = hint(GOV_HEAD).encode("c", &relayer).unwrap();
    assert!(bytes.len() <= HEAD_ADVANCED_MAX_BYTES);
    let public = relayer.verifying_key().to_bytes();
    assert_eq!(
        decode_head_advanced(&bytes, "c", &public).unwrap(),
        hint(GOV_HEAD)
    );
}

/// The frontier (3), the writer's commit point, is a hinted subkey.
#[test]
fn a_frontier_hint_round_trips() {
    let relayer = SigningKey::from_bytes(&[3; 32]);
    let bytes = hint(FRONTIER).encode("c", &relayer).unwrap();
    let public = relayer.verifying_key().to_bytes();
    assert_eq!(
        decode_head_advanced(&bytes, "c", &public).unwrap(),
        hint(FRONTIER)
    );
}

/// The routes (6) are hinted (plan E3.4 S4): a member's new route
/// reaches the mesh by the ring's hint, not by gossip presence.
#[test]
fn a_routes_hint_round_trips() {
    let relayer = SigningKey::from_bytes(&[3; 32]);
    let bytes = hint(ROUTES).encode("c", &relayer).unwrap();
    let public = relayer.verifying_key().to_bytes();
    assert_eq!(
        decode_head_advanced(&bytes, "c", &public).unwrap(),
        hint(ROUTES)
    );
}

#[test]
fn another_relayer_or_community_fails_and_unrelayed_subkeys_fail() {
    let relayer = SigningKey::from_bytes(&[3; 32]);
    let bytes = hint(OUTBOX).encode("c", &relayer).unwrap();
    let other = SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes();
    assert!(decode_head_advanced(&bytes, "c", &other).is_err());
    let public = relayer.verifying_key().to_bytes();
    assert!(decode_head_advanced(&bytes, "d", &public).is_err());
    for subkey in [0, 5, 7] {
        assert!(hint(subkey).encode("c", &relayer).is_err(), "{subkey}");
    }
}
