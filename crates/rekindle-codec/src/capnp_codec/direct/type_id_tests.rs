use super::{Carrier, DmTypeId};

#[test]
fn every_kind_round_trips_and_ids_are_ordered_and_unique() {
    for kind in DmTypeId::ALL {
        assert_eq!(DmTypeId::from_u16(kind.as_u16()), Some(*kind));
    }
    assert!(DmTypeId::ALL
        .windows(2)
        .all(|w| w[0].as_u16() < w[1].as_u16()));
}

/// Ids §6 allocates to later steps, or never, are not kinds yet.
#[test]
fn unallocated_ids_are_not_kinds() {
    for id in [
        // 0x0102 is FriendReject, retired (LV23 review L2).
        // 0x0127-0x012B are the lobby kinds (plan E7.2 G6); 0x012C is next.
        0x0000, 0x0102, 0x0109, 0x012C, 0x0150, 0x0195, 0x0207, 0x0230, 0x0300, 0x0402, 0xFFFF,
    ] {
        assert_eq!(DmTypeId::from_u16(id), None, "{id:#06x}");
    }
}

/// §6's Sealing column.
#[test]
fn carriers_follow_the_ledger() {
    let sealed: Vec<u16> = DmTypeId::ALL
        .iter()
        .filter(|k| k.carrier() == Carrier::Sealed)
        .map(|k| k.as_u16())
        .collect();
    assert_eq!(
        sealed,
        [0x0002, 0x0005, 0x0007, 0x0009, 0x0310, 0x0311, 0x0312]
    );
    assert_eq!(DmTypeId::PreKeyInit.carrier(), Carrier::PreKeyInit);
    assert_eq!(DmTypeId::GroupContent.carrier(), Carrier::GroupChain);
    let chain: Vec<u16> = DmTypeId::ALL
        .iter()
        .filter(|k| k.carrier() == Carrier::GroupChain)
        .map(|k| k.as_u16())
        .collect();
    assert_eq!(
        chain,
        [0x0124, 0x0126, 0x0127, 0x0128, 0x0129, 0x012A, 0x012B]
    );
    assert_eq!(DmTypeId::FriendAccept.carrier(), Carrier::FirstPayload);
    assert_eq!(
        DmTypeId::SessionResetAccept.carrier(),
        Carrier::FirstPayload
    );
}

/// G1: only the accepts, the content kinds and the pairwise group
/// control kinds open a session.
#[test]
fn the_first_payloads_are_an_allowlist() {
    let first: Vec<u16> = DmTypeId::ALL
        .iter()
        .filter(|k| k.is_first_payload())
        .map(|k| k.as_u16())
        .collect();
    assert_eq!(
        first,
        [0x0008, 0x0100, 0x0101, 0x0110, 0x0111, 0x0112, 0x0120, 0x0121, 0x0122, 0x0123]
    );
    for kind in [
        DmTypeId::Unfriended,
        DmTypeId::UnfriendedAck,
        DmTypeId::RelayEnvelope,
        DmTypeId::IdentityRotated,
        DmTypeId::Typing,
        DmTypeId::CallInvite,
        DmTypeId::PreKeyInit,
        DmTypeId::FriendRequest,
        DmTypeId::GroupContent,
        DmTypeId::StreamRecord,
    ] {
        assert!(!kind.is_first_payload(), "{kind:?}");
    }
}

/// G10: typing and the call and video kinds are ephemeral; no first
/// payload or sealed kind is.
#[test]
fn ephemeral_kinds_are_typing_calls_and_video() {
    let ephemeral: Vec<u16> = DmTypeId::ALL
        .iter()
        .filter(|k| k.is_ephemeral())
        .map(|k| k.as_u16())
        .collect();
    assert_eq!(
        ephemeral,
        [
            0x0200, 0x0201, 0x0202, 0x0203, 0x0204, 0x0205, 0x0206, 0x0210, 0x0211, 0x0212, 0x0213,
            0x0214, 0x0220, 0x0401
        ]
    );
    assert!(DmTypeId::ALL
        .iter()
        .filter(|k| k.is_ephemeral())
        .all(|k| !k.is_first_payload() && k.carrier() == Carrier::Ratchet));
}
