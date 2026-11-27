use super::*;

const EVERY_STATUS: [Status; 4] = [
    None,
    Some(FriendshipState::PendingOut),
    Some(FriendshipState::Accepted),
    Some(FriendshipState::Removing),
];

#[test]
fn a_refusal_is_for_now_only_while_our_request_is_outstanding() {
    for status in EVERY_STATUS {
        assert_eq!(
            not_yet(status),
            status == Some(FriendshipState::PendingOut),
            "{status:?}"
        );
    }
    // The real-app race: an invite opening a session from a sender we
    // asked is refused for now, never admitted early; from a stranger
    // it stays refused for good.
    let asked = Some(FriendshipState::PendingOut);
    assert!(!may_open_session(DmTypeId::GroupDmInvite, asked, false));
    assert!(!sender_admitted(DmTypeId::GroupDmInvite, asked));
    assert!(not_yet(asked));
    assert!(!may_open_session(DmTypeId::GroupDmInvite, None, false));
    assert!(!not_yet(None));
}

#[test]
fn friend_accept_reaches_the_requester() {
    assert!(sender_admitted(
        DmTypeId::FriendAccept,
        Some(FriendshipState::PendingOut)
    ));
    // The higher side of a crossing request is already Accepted.
    assert!(sender_admitted(
        DmTypeId::FriendAccept,
        Some(FriendshipState::Accepted)
    ));
}

#[test]
fn friend_accept_from_a_stranger_or_a_removed_row_drops() {
    assert!(!sender_admitted(DmTypeId::FriendAccept, None));
    assert!(!sender_admitted(
        DmTypeId::FriendAccept,
        Some(FriendshipState::Removing)
    ));
}

#[test]
fn a_pending_sender_may_send_nothing_but_the_accept() {
    for status in [
        None,
        Some(FriendshipState::PendingOut),
        Some(FriendshipState::Removing),
    ] {
        assert!(!sender_admitted(DmTypeId::DmText, status));
        assert!(!sender_admitted(DmTypeId::SessionResetRequest, status));
    }
    assert!(sender_admitted(
        DmTypeId::DmText,
        Some(FriendshipState::Accepted)
    ));
}

#[test]
fn a_request_and_a_removal_come_from_anyone_not_blocked() {
    for kind in [
        DmTypeId::FriendRequest,
        DmTypeId::Unfriended,
        DmTypeId::UnfriendedAck,
    ] {
        assert!(sender_admitted(kind, None));
        assert!(sender_admitted(kind, Some(FriendshipState::Removing)));
    }
}

#[test]
fn a_stranger_opens_no_session_with_any_kind() {
    for kind in [
        DmTypeId::FriendAccept,
        DmTypeId::SessionResetAccept,
        DmTypeId::DmText,
        DmTypeId::DmEdit,
        DmTypeId::DmDelete,
        DmTypeId::DmReaction,
        DmTypeId::Unfriended,
        DmTypeId::UnfriendedAck,
        DmTypeId::RelayEnvelope,
        DmTypeId::FriendRequest,
    ] {
        for reset in [false, true] {
            assert!(!may_open_session(kind, None, reset), "{kind:?}");
            assert!(
                !may_open_session(kind, Some(FriendshipState::Removing), reset),
                "{kind:?}"
            );
        }
    }
}

#[test]
fn each_first_payload_opens_a_session_only_under_its_rule() {
    for status in EVERY_STATUS {
        let accepted = status == Some(FriendshipState::Accepted);
        let asked = status == Some(FriendshipState::PendingOut);
        assert_eq!(
            may_open_session(DmTypeId::FriendAccept, status, false),
            accepted || asked
        );
        assert!(!may_open_session(
            DmTypeId::SessionResetAccept,
            status,
            false
        ));
        assert_eq!(
            may_open_session(DmTypeId::SessionResetAccept, status, true),
            accepted
        );
        assert_eq!(may_open_session(DmTypeId::DmText, status, false), accepted);
        // The removal kinds a stranger may send sealed never open one.
        assert!(!may_open_session(DmTypeId::Unfriended, status, true));
        assert!(!may_open_session(DmTypeId::RelayEnvelope, status, true));
    }
}
