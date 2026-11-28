use rekindle_codec::capnp_codec::direct::DmTypeId;

use super::*;
use crate::group::tests::{member_of, signer};

fn roster() -> Roster {
    Roster::new((1..=3).map(|i| member_of(&signer(i))).collect()).expect("roster")
}

#[test]
fn admission_table() {
    let roster = roster();
    let gid = [5u8; 16];
    let other_gid = [6u8; 16];
    let groups = [AcceptedGroup {
        group_id: &gid,
        roster: &roster,
    }];
    let bea = roster.members()[1].clone();
    let stranger = member_of(&signer(9)).identity;
    let kind = |kind, group_id| GroupFrame::Kind { kind, group_id };

    for k in [
        DmTypeId::GroupDmInvite,
        DmTypeId::GroupDmAccept,
        DmTypeId::GroupDmDecline,
        DmTypeId::GroupDmLeave,
        DmTypeId::GroupContent,
    ] {
        assert_eq!(
            admit(&bea.identity, false, &kind(k, Some(&gid)), &groups),
            Admission::Admit
        );
        assert_eq!(
            admit(&bea.identity, true, &kind(k, Some(&gid)), &groups),
            Admission::Blocked
        );
        assert_eq!(
            admit(&stranger, false, &kind(k, Some(&gid)), &groups),
            Admission::NotOnRoster
        );
        assert_eq!(
            admit(&bea.identity, false, &kind(k, Some(&other_gid)), &groups),
            Admission::NotOnRoster
        );
        assert_eq!(
            admit(&bea.identity, false, &kind(k, None), &groups),
            Admission::NotOnRoster
        );
    }
    for k in [
        DmTypeId::FriendRequest,
        DmTypeId::FriendAccept,
        DmTypeId::FriendsKey,
        DmTypeId::DmText,
        DmTypeId::ProfileKeyRotated,
    ] {
        assert_eq!(
            admit(&bea.identity, false, &kind(k, Some(&gid)), &groups),
            Admission::KindRefused
        );
    }

    let own_chain = GroupFrame::StreamRecord {
        group_id: Some(&gid),
    };
    assert_eq!(
        admit(&bea.identity, false, &own_chain, &groups),
        Admission::Admit
    );
    assert_eq!(
        admit(&"f".repeat(64), false, &own_chain, &groups),
        Admission::NotOnRoster
    );
    let dm_chain = GroupFrame::StreamRecord { group_id: None };
    assert_eq!(
        admit(&bea.identity, false, &dm_chain, &groups),
        Admission::KindRefused
    );

    let accept = kind(DmTypeId::GroupDmAccept, Some(&gid));
    let init = GroupFrame::PreKeyInit { first: &accept };
    assert_eq!(
        admit(&bea.identity, false, &init, &groups),
        Admission::Admit
    );
    assert_eq!(
        admit(&bea.identity, true, &init, &groups),
        Admission::Blocked
    );
    let friend = kind(DmTypeId::FriendAccept, None);
    let friend_init = GroupFrame::PreKeyInit { first: &friend };
    assert_eq!(
        admit(&bea.identity, false, &friend_init, &groups),
        Admission::KindRefused
    );

    // Leaving deletes the group; a member who left is off the roster.
    assert_eq!(
        admit(&bea.identity, false, &accept, &[]),
        Admission::NotOnRoster
    );
    let shrunk = roster.without(&bea.identity).expect("on it");
    let after = [AcceptedGroup {
        group_id: &gid,
        roster: &shrunk,
    }];
    assert_eq!(
        admit(&bea.identity, false, &accept, &after),
        Admission::NotOnRoster
    );
}

/// Plan E7.2 G6: a lobby kind opened from a group entry is admitted only
/// from a writer on that group's roster now; never as a frame of its own,
/// never as a `PreKeyInit`'s first payload, and never a non-lobby kind.
#[test]
fn a_lobby_kind_is_admitted_only_from_the_roster() {
    let roster = roster();
    let gid = [5u8; 16];
    let other_gid = [6u8; 16];
    let groups = [AcceptedGroup {
        group_id: &gid,
        roster: &roster,
    }];
    let bea = roster.members()[1].identity.clone();
    let stranger = member_of(&signer(9)).identity;
    let shrunk = roster.without(&bea).expect("on it");
    let after = [AcceptedGroup {
        group_id: &gid,
        roster: &shrunk,
    }];
    for kind in [
        DmTypeId::LobbyOpen,
        DmTypeId::LobbyJoin,
        DmTypeId::LobbyLeave,
        DmTypeId::LobbyLaunch,
        DmTypeId::LobbyClose,
    ] {
        assert!(is_lobby_kind(kind));
        let lobby = GroupFrame::Lobby {
            kind,
            group_id: &gid,
        };
        assert_eq!(admit(&bea, false, &lobby, &groups), Admission::Admit);
        assert_eq!(admit(&bea, true, &lobby, &groups), Admission::Blocked);
        assert_eq!(
            admit(&stranger, false, &lobby, &groups),
            Admission::NotOnRoster,
            "{kind:?} from a writer never on the roster"
        );
        assert_eq!(
            admit(&bea, false, &lobby, &after),
            Admission::NotOnRoster,
            "{kind:?} from a member who left"
        );
        let elsewhere = GroupFrame::Lobby {
            kind,
            group_id: &other_gid,
        };
        assert_eq!(
            admit(&bea, false, &elsewhere, &groups),
            Admission::NotOnRoster
        );
        let frame = GroupFrame::Kind {
            kind,
            group_id: Some(&gid),
        };
        assert_eq!(admit(&bea, false, &frame, &groups), Admission::KindRefused);
        let init = GroupFrame::PreKeyInit { first: &lobby };
        assert_eq!(admit(&bea, false, &init, &groups), Admission::KindRefused);
    }
    let text = GroupFrame::Lobby {
        kind: DmTypeId::DmText,
        group_id: &gid,
    };
    assert_eq!(admit(&bea, false, &text, &groups), Admission::KindRefused);
}
