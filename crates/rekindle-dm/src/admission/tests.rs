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
        record_key: &bea.participant_record_key,
        group_id: Some(&gid),
    };
    assert_eq!(
        admit(&bea.identity, false, &own_chain, &groups),
        Admission::Admit
    );
    let anothers_chain = GroupFrame::StreamRecord {
        record_key: &roster.members()[2].participant_record_key,
        group_id: Some(&gid),
    };
    assert_eq!(
        admit(&bea.identity, false, &anothers_chain, &groups),
        Admission::NotOnRoster
    );
    let dm_chain = GroupFrame::StreamRecord {
        record_key: &bea.participant_record_key,
        group_id: None,
    };
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
