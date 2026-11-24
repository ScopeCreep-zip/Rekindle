//! The `LobbyGet` answer (plan E7.2 G6, SK2-G §5) in the golden DTO file:
//! `Option<LobbyView>`, one sample per `LobbyStatus` and per verdict shape.

use rekindle_types::dto::group_lobby::{LobbyRefusal, LobbyStatus, LobbyVerdict, LobbyView};
use rekindle_types::game_server::ServerAddress;
use serde_json::Value;

fn view(pk: &str, state: LobbyStatus, verdicts: [LobbyVerdict; 4]) -> LobbyView {
    let [join, leave, launch, close] = verdicts;
    LobbyView {
        lobby_id: "ab".repeat(16),
        host_key: pk.to_owned(),
        game_id: 4181,
        members: vec!["d".repeat(64)],
        state,
        join,
        leave,
        launch,
        close,
    }
}

pub fn samples(add: &mut impl FnMut(&str, Value), pk: &str) {
    let refused = LobbyVerdict::refused;
    let open = view(
        pk,
        LobbyStatus::Open,
        [
            LobbyVerdict::Allowed,
            refused(LobbyRefusal::NotIn),
            refused(LobbyRefusal::NotHost),
            refused(LobbyRefusal::NotHost),
        ],
    );
    let launched = view(
        pk,
        LobbyStatus::Launched {
            server: ServerAddress::parse("203.0.113.7:27015").unwrap(),
        },
        [
            refused(LobbyRefusal::NotOpen),
            refused(LobbyRefusal::OwnLobby),
            refused(LobbyRefusal::NotOpen),
            LobbyVerdict::Allowed,
        ],
    );
    let closed = view(
        pk,
        LobbyStatus::Closed,
        [
            refused(LobbyRefusal::NotOpen),
            refused(LobbyRefusal::Closed),
            refused(LobbyRefusal::AlreadyIn),
            refused(LobbyRefusal::NoMembers),
        ],
    );
    add("lobbyGet.open", serde_json::to_value(Some(open)).unwrap());
    add(
        "lobbyGet.launched",
        serde_json::to_value(Some(launched)).unwrap(),
    );
    add(
        "lobbyGet.closed",
        serde_json::to_value(Some(closed)).unwrap(),
    );
    add(
        "lobbyGet.none",
        serde_json::to_value(None::<LobbyView>).unwrap(),
    );
}
