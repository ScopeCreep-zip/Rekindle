use super::*;

#[test]
fn server_addresses_are_host_and_port_only() {
    for good in [
        "1.2.3.4:27015",
        "[::1]:27015",
        "[2001:db8::7]:16567",
        "play.example.com:25565",
        "localhost:1",
    ] {
        assert_eq!(ServerAddress::parse(good).unwrap().as_str(), good);
    }
    for bad in [
        "1.2.3.4/../run/1",
        "1.2.3.4:27015 +exec x",
        "host?x",
        "steam://connect/1.2.3.4:27015",
        "1.2.3.4:27015/x",
        "1.2.3.4:27015#x",
        "1.2.3.4:%32",
        "1.2.3.4",
        "1.2.3.4:0",
        "1.2.3.4:65536",
        "::1:27015",
        "[::1]",
        "\"host\":1",
        "a..b:1",
        "+connect:1",
        "",
    ] {
        assert!(ServerAddress::parse(bad).is_err(), "{bad} was accepted");
    }
    let long_label = format!("{}.com:1", "a".repeat(64));
    assert!(ServerAddress::parse(&long_label).is_err());
}

#[test]
fn a_server_address_decodes_only_when_valid() {
    assert!(serde_json::from_str::<ServerAddress>("\"1.2.3.4:1\"").is_ok());
    assert!(serde_json::from_str::<ServerAddress>("\"1.2.3.4:1 x\"").is_err());
}

#[test]
fn server_ids_are_gs_and_sixteen_hex() {
    assert!(is_server_id("gs_0123456789abcdef"));
    for bad in [
        "gs_0123",
        "0123456789abcdef",
        "gs_0123456789ABCDEF",
        "gs_0123456789abcdeg",
        "",
    ] {
        assert!(!is_server_id(bad), "{bad} was accepted");
    }
}

#[test]
fn game_server_info_roundtrip() {
    let g = GameServerInfo {
        id: "gs_01".into(),
        game_id: "halo".into(),
        label: "Friday CTF".into(),
        address: "10.0.0.1:27015".into(),
        added_by: "abcd".into(),
        created_at: 42,
    };
    let json = serde_json::to_string(&g).unwrap();
    let back: GameServerInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(g, back);
}
