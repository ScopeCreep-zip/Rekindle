use super::*;

#[test]
fn only_an_accepted_friend_with_our_setting_on() {
    assert!(allowed(true, Some(FriendshipState::Accepted)));
    assert!(
        !allowed(false, Some(FriendshipState::Accepted)),
        "setting off"
    );
    // A request to us (PendingIn) and a blocked peer hold no friend
    // row: like a stranger, they have no friendship.
    assert!(!allowed(true, None), "non-friend, PendingIn or blocked");
    for state in [FriendshipState::PendingOut, FriendshipState::Removing] {
        assert!(!allowed(true, Some(state)), "{state:?}");
        assert!(!allowed(false, Some(state)), "{state:?}");
    }
}
