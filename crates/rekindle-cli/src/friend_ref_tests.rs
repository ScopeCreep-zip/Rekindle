use rekindle_types::dto::friends::Friend;
use rekindle_types::friend::{FriendshipState, UserStatus};

const KEY_ALICE: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const KEY_BOB: &str = "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";
const KEY_CAROL: &str = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";

fn friend(key: &str, name: &str, nickname: Option<&str>) -> Friend {
    Friend {
        public_key: key.to_owned(),
        display_name: name.to_owned(),
        nickname: nickname.map(str::to_owned),
        status: UserStatus::Offline,
        status_message: None,
        game_info: None,
        group_id: None,
        group: None,
        group_label: String::new(),
        last_seen_at: None,
        friendship_state: FriendshipState::Accepted,
        handshake_error: None,
        handshake_retrying: false,
        identity_pin_mismatch: false,
        group_dm_block: None,
    }
}

fn list() -> Vec<Friend> {
    vec![
        friend(KEY_ALICE, "alice", Some("Al")),
        friend(KEY_BOB, "kali", None),
        friend(KEY_CAROL, "kali", Some("work")),
    ]
}

#[test]
fn a_name_or_nickname_of_one_friend_is_their_key() {
    let friends = list();
    assert_eq!(super::pick(&friends, "alice").unwrap(), KEY_ALICE);
    assert_eq!(super::pick(&friends, "Al").unwrap(), KEY_ALICE);
    assert_eq!(super::pick(&friends, "work").unwrap(), KEY_CAROL);
    assert_eq!(super::pick(&friends, KEY_BOB).unwrap(), KEY_BOB);
}

#[test]
fn a_shared_name_is_refused_naming_every_match() {
    let err = super::pick(&list(), "kali").unwrap_err().to_string();
    assert!(err.contains("2 friends are named \"kali\""), "{err}");
    assert!(err.contains(KEY_BOB) && err.contains(KEY_CAROL), "{err}");
    assert!(!err.contains(KEY_ALICE), "{err}");
}

#[test]
fn an_unknown_or_inexact_name_is_refused() {
    for given in ["bob", "Alice", "al", "kal"] {
        let err = super::pick(&list(), given).unwrap_err().to_string();
        assert!(err.contains("no friend is named"), "{given}: {err}");
    }
}

#[test]
fn a_friend_whose_name_and_nickname_match_is_one_match() {
    let friends = vec![friend(KEY_ALICE, "alice", Some("alice"))];
    assert_eq!(super::pick(&friends, "alice").unwrap(), KEY_ALICE);
}

#[test]
fn only_sixty_four_hex_is_a_key() {
    assert!(super::is_key(KEY_ALICE));
    assert!(!super::is_key("kali"));
    assert!(!super::is_key(&"a".repeat(63)));
    assert!(!super::is_key(&format!("{}g", "a".repeat(63))));
    assert!(!super::is_key(&"a".repeat(65)));
}
