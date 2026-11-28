use super::*;
use crate::capnp_codec::direct::{DirectBody, DmTypeId};

/// Plan E7.2: a `ProfileBio` past 190 characters or with a control
/// character is refused on decode; a blank one reads as no bio.
#[test]
fn bad_bios_are_refused_and_blank_is_none() {
    for bad in ["x".repeat(191), "a\u{1b}b".to_string()] {
        let bytes = DirectBody::ProfileBio(ProfileBio { bio: Some(bad) }).encode();
        assert!(DirectBody::decode(DmTypeId::ProfileBio, &bytes.unwrap()).is_err());
    }
    let bytes = DirectBody::ProfileBio(ProfileBio {
        bio: Some(" ".into()),
    })
    .encode();
    assert_eq!(
        DirectBody::decode(DmTypeId::ProfileBio, &bytes.unwrap()).unwrap(),
        DirectBody::ProfileBio(ProfileBio { bio: None })
    );
}

/// Plan E7.2: a `GameInvite` whose server is not a plain host:port (a
/// URI, a launch argument, no port) or that names game 0 is refused on
/// decode, so it is never shown or launched.
#[test]
fn bad_game_invites_are_refused() {
    let bytes = |game_id: u32, server: &str| {
        crate::capnp_codec::direct::encode_root::<game_invite::Owned>(|mut b| {
            b.set_game_id(game_id);
            b.set_server(server);
            b.set_expires_at_ms(123);
            Ok(())
        })
    };
    for server in [
        "steam://connect/1.2.3.4:27015",
        "1.2.3.4:27015 +exec x",
        "1.2.3.4",
    ] {
        assert!(DirectBody::decode(DmTypeId::GameInvite, &bytes(7, server).unwrap()).is_err());
    }
    assert!(DirectBody::decode(DmTypeId::GameInvite, &bytes(0, "1.2.3.4:27015").unwrap()).is_err());
    assert_eq!(
        DirectBody::decode(DmTypeId::GameInvite, &bytes(7, "1.2.3.4:27015").unwrap()).unwrap(),
        DirectBody::GameInvite(GameInvite {
            game_id: 7,
            server: ServerAddress::parse("1.2.3.4:27015").unwrap(),
            expires_at_ms: 123,
        })
    );
}

/// Plan E7.2: a `GameTotals` naming game 0, a game twice, or more games
/// than the cap is refused on decode.
#[test]
fn bad_game_totals_are_refused() {
    let t = |game_id| GameTotal {
        game_id,
        total_secs: 1,
        last_played: 1,
    };
    for totals in [
        vec![t(0)],
        vec![t(7), t(7)],
        (1..=u32::try_from(MAX_GAME_TOTALS).unwrap() + 1)
            .map(t)
            .collect(),
    ] {
        let bytes = DirectBody::GameTotals(GameTotals { totals }).encode();
        assert!(DirectBody::decode(DmTypeId::GameTotals, &bytes.unwrap()).is_err());
    }
}
