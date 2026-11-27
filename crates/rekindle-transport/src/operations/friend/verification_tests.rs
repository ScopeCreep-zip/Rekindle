/// Withdrawing and refusing go through an unacknowledged key change;
/// content, typing and session openers do not.
#[test]
fn only_withdrawals_skip_the_key_change_hold() {
    use rekindle_codec::capnp_codec::direct::DmTypeId;
    for kind in [
        DmTypeId::Unfriended,
        DmTypeId::UnfriendedAck,
        DmTypeId::SessionResetDecline,
        DmTypeId::PreKeyRejected,
    ] {
        assert!(super::is_protective(kind), "{kind:?}");
    }
    for kind in [
        DmTypeId::DmText,
        DmTypeId::Typing,
        DmTypeId::PreKeyInit,
        DmTypeId::CallInvite,
    ] {
        assert!(!super::is_protective(kind), "{kind:?}");
    }
}
