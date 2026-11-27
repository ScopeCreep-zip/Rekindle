use rekindle_types::friend::{FriendState, FriendshipState};

use super::recipients;

fn friend(key: &str, state: FriendshipState) -> FriendState {
    FriendState::new(key.into(), key.into(), state)
}

/// Plan E2.5: with two accepted friends, removing one sends the new
/// epoch to the other only, whether the removed row is still
/// `Accepted` (a block, an unfriend received) or already `Removing`;
/// a pending friend gets nothing.
#[test]
fn the_rotated_key_never_reaches_the_removed_friend() {
    for removed_state in [FriendshipState::Accepted, FriendshipState::Removing] {
        let friends = vec![
            friend("alice", FriendshipState::Accepted),
            friend("bob", removed_state),
            friend("carol", FriendshipState::PendingOut),
        ];
        assert_eq!(recipients(friends, "bob"), ["alice"]);
    }
}
