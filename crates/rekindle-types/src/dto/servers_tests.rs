use super::*;

/// The wire shapes the frontend mirrors; an impossible pair is refused.
#[test]
fn verdicts_are_exact_on_the_wire() {
    let refused = ServerVerdict::refused(ServerRefusal::AlreadyFavourite);
    assert_eq!(
        serde_json::to_string(&ServerVerdict::Allowed).unwrap(),
        r#"{"allowed":true}"#
    );
    assert_eq!(
        serde_json::to_string(&refused).unwrap(),
        r#"{"allowed":false,"reason":"alreadyFavourite"}"#
    );
    assert!(serde_json::from_str::<ServerVerdict>(r#"{"allowed":false}"#).is_err());
    assert!(
        serde_json::from_str::<ServerVerdict>(r#"{"allowed":true,"reason":"badAddress"}"#).is_err()
    );
    let join = ServerJoin {
        game_id: Some(4100),
        server: Some("1.2.3.4:27015".into()),
        launch_source: JoinSource::Recent,
        joinability: Joinability::of(Some(NotJoinableReason::CannotLaunch)),
    };
    let text = serde_json::to_string(&join).unwrap();
    assert_eq!(
        text,
        r#"{"gameId":4100,"server":"1.2.3.4:27015","launchSource":"recent","joinable":false,"joinableReason":"cannotLaunch"}"#
    );
    assert_eq!(serde_json::from_str::<ServerJoin>(&text).unwrap(), join);
    assert!(serde_json::from_str::<ServerJoin>(
        r#"{"gameId":1,"server":null,"launchSource":"recent","joinable":false}"#
    )
    .is_err());
}

/// `ServerInfo`'s tagged wire shapes, as the frontend mirrors them.
#[test]
fn server_info_is_tagged_by_kind() {
    let answered = ServerInfo::Answered {
        name: "Dust".into(),
        map: "de_dust".into(),
        players: 5,
        max_players: 16,
        ping_ms: 31,
    };
    assert_eq!(
        serde_json::to_string(&answered).unwrap(),
        r#"{"kind":"answered","name":"Dust","map":"de_dust","players":5,"maxPlayers":16,"pingMs":31}"#
    );
    assert_eq!(
        serde_json::to_string(&ServerInfo::NotQueried).unwrap(),
        r#"{"kind":"notQueried"}"#
    );
    assert_eq!(
        serde_json::to_string(&ServerVerdict::refused(ServerRefusal::Refreshing)).unwrap(),
        r#"{"allowed":false,"reason":"refreshing"}"#
    );
}
