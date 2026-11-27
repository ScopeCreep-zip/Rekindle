use super::*;

#[test]
fn parse_status_legacy_and_9byte_forms() {
    assert_eq!(parse_status(&[]), None);
    assert_eq!(parse_status(&[0]), Some(UserStatusKind::Online));
    assert_eq!(parse_status(&[1]), Some(UserStatusKind::Away));
    assert_eq!(parse_status(&[2]), Some(UserStatusKind::Busy));
    assert_eq!(parse_status(&[3]), Some(UserStatusKind::Offline));
    assert_eq!(parse_status(&[7]), Some(UserStatusKind::Offline));
    // Wide payload still reads byte 0.
    let mut wide = vec![1u8];
    wide.extend_from_slice(&0i64.to_be_bytes());
    assert_eq!(parse_status(&wide), Some(UserStatusKind::Away));
}

#[test]
fn parse_status_timestamp_requires_9_bytes() {
    assert_eq!(parse_status_timestamp(&[0]), None);
    assert_eq!(parse_status_timestamp(&[0u8; 8]), None);
    let mut payload = vec![1u8];
    payload.extend_from_slice(&123_456_789_i64.to_be_bytes());
    assert_eq!(parse_status_timestamp(&payload), Some(123_456_789));
}

#[test]
fn status_to_wire_byte_maps_invisible_as_offline() {
    assert_eq!(status_to_wire_byte(UserStatusKind::Online), 0);
    assert_eq!(status_to_wire_byte(UserStatusKind::Away), 1);
    assert_eq!(status_to_wire_byte(UserStatusKind::Busy), 2);
    assert_eq!(status_to_wire_byte(UserStatusKind::Offline), 3);
    assert_eq!(status_to_wire_byte(UserStatusKind::Invisible), 3);
}

/// Subkey 4 is the capnp `GameStatus`; an address that is not
/// `host:port` is dropped, the game kept; empty is no game.
#[test]
fn game_info_decodes_the_capnp_game_status() {
    use rekindle_codec::capnp_codec::presence::encode_game_status;
    use rekindle_codec::capnp_codec::presence::GameInfo;
    let game = |server_address: &str| GameInfo {
        game_id: 4181,
        game_name: "Counter-Strike 2".into(),
        server_info: None,
        elapsed_seconds: 90,
        server_address: Some(server_address.into()),
    };
    let parsed = parse_game_info(&encode_game_status(&game("1.2.3.4:27015")).unwrap()).unwrap();
    assert_eq!(parsed.game_id, 4181);
    assert_eq!(parsed.game_name, "Counter-Strike 2");
    assert_eq!(parsed.elapsed_seconds, 90);
    assert_eq!(parsed.server_address.as_deref(), Some("1.2.3.4:27015"));

    let hostile =
        parse_game_info(&encode_game_status(&game("1.2.3.4:27015 +exec x")).unwrap()).unwrap();
    assert_eq!(hostile.server_address, None);
    assert_eq!(hostile.game_name, "Counter-Strike 2");

    assert!(parse_game_info(&[]).is_none());
    assert!(
        parse_game_info(br#"{"game_id":1}"#).is_none(),
        "JSON is not the wire"
    );
}
