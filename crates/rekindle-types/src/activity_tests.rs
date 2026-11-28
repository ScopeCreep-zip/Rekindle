use super::*;

/// The wire shape is the IPC enum convention, and `name()` is the
/// serde tag, so the `kind` column and the payload agree.
#[test]
fn the_tag_is_the_name_and_fields_are_camel_case() {
    let kind = ActivityKind::StartedGame {
        friend: "ab".into(),
        game_id: 7,
        server: Some("203.0.113.7:27015".into()),
    };
    let json = serde_json::to_value(&kind).unwrap();
    assert_eq!(json["type"], kind.name());
    assert_eq!(json["data"]["gameId"], 7);
    assert_eq!(kind.friend(), Some("ab"));
    let own = ActivityKind::JoinedCommunity {
        community: "c".into(),
    };
    assert_eq!(serde_json::to_value(&own).unwrap()["type"], own.name());
    assert_eq!(own.friend(), None);
}
