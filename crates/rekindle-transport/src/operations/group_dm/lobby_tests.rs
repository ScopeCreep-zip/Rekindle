//! Each lobby verdict is the Host's, decided from the fold (SK2-G §5 (5)).

use super::*;

fn row(state: LobbyState, host: &str, members: &[&str]) -> LobbyRow {
    LobbyRow {
        lobby_id: "ab".repeat(16),
        host_key: host.into(),
        game_id: 4181,
        state,
        server: (state == LobbyState::Launched).then(|| "203.0.113.7:27015".into()),
        opened_at: 1,
        members: members.iter().map(|m| (*m).to_owned()).collect(),
    }
}

#[test]
fn join_is_an_open_lobby_another_member_hosts_that_we_are_not_in() {
    let v = view_of(row(LobbyState::Open, "h", &[]), "me").unwrap();
    assert_eq!(v.join, V::Allowed);
    let v = view_of(row(LobbyState::Open, "h", &["me"]), "me").unwrap();
    assert_eq!(v.join, V::refused(R::AlreadyIn));
    let v = view_of(row(LobbyState::Open, "me", &[]), "me").unwrap();
    assert_eq!(v.join, V::refused(R::OwnLobby));
    let v = view_of(row(LobbyState::Launched, "h", &[]), "me").unwrap();
    assert_eq!(v.join, V::refused(R::NotOpen));
}

#[test]
fn leave_is_in_it_and_not_the_host() {
    assert_eq!(
        view_of(row(LobbyState::Open, "h", &["me"]), "me")
            .unwrap()
            .leave,
        V::Allowed
    );
    assert_eq!(
        view_of(row(LobbyState::Open, "h", &[]), "me")
            .unwrap()
            .leave,
        V::refused(R::NotIn)
    );
    assert_eq!(
        view_of(row(LobbyState::Open, "me", &["x"]), "me")
            .unwrap()
            .leave,
        V::refused(R::OwnLobby)
    );
}

#[test]
fn launch_is_the_host_of_an_open_lobby_with_a_member_and_close_the_host() {
    let v = view_of(row(LobbyState::Open, "me", &["x"]), "me").unwrap();
    assert_eq!((v.launch, v.close), (V::Allowed, V::Allowed));
    let v = view_of(row(LobbyState::Open, "me", &[]), "me").unwrap();
    assert_eq!(v.launch, V::refused(R::NoMembers));
    let v = view_of(row(LobbyState::Open, "h", &["me"]), "me").unwrap();
    assert_eq!(
        (v.launch, v.close),
        (V::refused(R::NotHost), V::refused(R::NotHost))
    );
    let v = view_of(row(LobbyState::Launched, "me", &["x"]), "me").unwrap();
    assert_eq!(v.launch, V::refused(R::NotOpen));
    assert_eq!(
        v.state,
        LobbyStatus::Launched {
            server: ServerAddress::parse("203.0.113.7:27015").unwrap()
        }
    );
    let v = view_of(row(LobbyState::Closed, "me", &["x"]), "me").unwrap();
    assert_eq!(v.close, V::refused(R::Closed));
}
