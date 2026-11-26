use super::*;

#[test]
fn only_friend_control_kinds_go_on_the_chain() {
    for kind in [
        DmTypeId::FriendsKey,
        DmTypeId::GroupDmInvite,
        DmTypeId::GroupDmRosterGrow,
        DmTypeId::GroupDmAccept,
        DmTypeId::GroupDmDecline,
        DmTypeId::GroupDmLeave,
    ] {
        assert!(on_chain(kind), "{kind:?} rides the dm chain");
        assert_eq!(
            kind.carrier(),
            rekindle_codec::capnp_codec::direct::Carrier::Ratchet
        );
    }
    for kind in [
        DmTypeId::Unfriended,
        DmTypeId::UnfriendedAck,
        DmTypeId::CallEnd,
        DmTypeId::SessionResetRequest,
        DmTypeId::SessionResetAccept,
        DmTypeId::FriendRequest,
        DmTypeId::FriendAccept,
        DmTypeId::PreKeyInit,
    ] {
        assert!(!on_chain(kind), "{kind:?} stays off the dm chain");
    }
}
