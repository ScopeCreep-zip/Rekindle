//! The lobby kinds (plan E7.2 G6): each round-trips under its kind, their
//! bytes are pinned, and a launch whose server is not a `host:port`, a
//! lobby id of the wrong length or game 0 drops the frame.

use super::*;
use rekindle_types::game_server::ServerAddress;

const LOBBY: LobbyId = [0x1B; 16];

fn lobby_ref() -> LobbyRef {
    LobbyRef { lobby_id: LOBBY }
}

/// Each lobby kind, with the packed bytes its body encodes to.
fn goldens() -> Vec<(DirectBody, &'static str)> {
    vec![
        (
            DirectBody::LobbyOpen(LobbyOpen {
                lobby_id: LOBBY,
                game_id: 4181,
            }),
            "1005500101035510110182ff1b1b1b1b1b1b1b1b011b1b1b1b1b1b1b1b",
        ),
        (DirectBody::LobbyJoin(lobby_ref()), "10044001110182ff1b1b1b1b1b1b1b1b011b1b1b1b1b1b1b1b"),
        (DirectBody::LobbyLeave(lobby_ref()), "10044001110182ff1b1b1b1b1b1b1b1b011b1b1b1b1b1b1b1b"),
        (
            DirectBody::LobbyLaunch(LobbyLaunch {
                lobby_id: LOBBY,
                server: ServerAddress::parse("203.0.113.7:27015").unwrap(),
            }),
            "10084002110582110992ff1b1b1b1b1b1b1b1b031b1b1b1b1b1b1b1b3230332e302e3131332e373a323730310135",
        ),
        (DirectBody::LobbyClose(lobby_ref()), "10044001110182ff1b1b1b1b1b1b1b1b011b1b1b1b1b1b1b1b"),
    ]
}

#[test]
fn every_lobby_kind_round_trips_and_its_bytes_are_pinned() {
    let kinds: Vec<u16> = goldens()
        .iter()
        .map(|(b, _)| b.type_id().as_u16())
        .collect();
    assert_eq!(kinds, [0x0127, 0x0128, 0x0129, 0x012A, 0x012B]);
    for (body, golden) in goldens() {
        let bytes = body.encode();
        assert_eq!(hex::encode(&bytes), golden, "{:?}", body.type_id());
        assert_eq!(
            DirectBody::decode(body.type_id(), &bytes.unwrap()),
            Ok(body.clone()),
            "{:?}",
            body.type_id()
        );
        assert_eq!(body.type_id().carrier(), Carrier::GroupChain);
        assert!(!body.type_id().is_first_payload() && !body.type_id().is_ephemeral());
    }
}

/// A launch's server is parsed on decode: anything that is not a
/// `host:port` (a path, a query, a launch argument, no port) drops it.
#[test]
fn a_launch_whose_server_is_not_an_address_is_dropped() {
    for server in [
        "",
        "203.0.113.7",
        "203.0.113.7:0",
        "203.0.113.7:27015/connect",
        "host:27015?x=1",
        "host:27015 +exec evil.cfg",
        "steam://connect/203.0.113.7:27015",
    ] {
        let bytes = encode_root::<crate::direct_capnp::lobby_launch::Owned>(|mut b| {
            b.set_lobby_id(LOBBY.as_slice());
            b.set_server(server);
        });
        assert!(
            matches!(
                DirectBody::decode(DmTypeId::LobbyLaunch, &bytes.unwrap()),
                Err(CodecError::Deserialization(_))
            ),
            "{server:?}"
        );
    }
}

/// A lobby id of the wrong length and game 0 are malformed.
#[test]
fn a_short_lobby_id_or_game_zero_is_dropped() {
    let short = encode_root::<crate::direct_capnp::lobby_ref::Owned>(|mut b| {
        b.set_lobby_id([1; 15].as_slice());
    });
    for kind in [
        DmTypeId::LobbyJoin,
        DmTypeId::LobbyLeave,
        DmTypeId::LobbyClose,
    ] {
        assert!(DirectBody::decode(kind, &short.unwrap()).is_err(), "{kind:?}");
    }
    let game_zero = DirectBody::LobbyOpen(LobbyOpen {
        lobby_id: LOBBY,
        game_id: 0,
    });
    assert!(DirectBody::decode(DmTypeId::LobbyOpen, &game_zero.encode().unwrap()).is_err());
}
