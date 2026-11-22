//! `GameLaunch`'s class follows its `JoinSource` (lead decision (50)).
//! Only the selection and validation run: no game is ever launched here.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::game_server::{JoinSource, ServerAddress};
use rekindle_types::intent::{effective_class, Fact, IntentClass, IntentKind, Subject};

use crate::host::direct_tests::ann;
use crate::host::game_totals_tests::run;

fn launch(server: Option<&str>, source: JoinSource) -> IpcRequest {
    IpcRequest::GameLaunch {
        game_id: 4181,
        server: server.map(|s| ServerAddress::parse(s).unwrap()),
        source,
    }
}

async fn class_of(
    host: &crate::host::Host,
    request: &IpcRequest,
) -> Result<(IntentClass, rekindle_idempotency::intent_gate::Selection), IpcResponse> {
    let s = super::super::select(&host.ctx, request, IntentKind::LaunchGameToServer).await?;
    Ok((effective_class(s.kind, &s.escalations), s))
}

#[tokio::test]
async fn a_peer_supplied_server_is_trust_changing_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    for source in [
        JoinSource::FriendPresence,
        JoinSource::Invite,
        JoinSource::Community,
    ] {
        let (class, s) = class_of(&host, &launch(Some("1.2.3.4:27015"), source))
            .await
            .unwrap();
        assert_eq!(class, IntentClass::TrustChanging, "{source:?}");
        assert_eq!(s.kind, IntentKind::JoinGameServer);
        assert_eq!(
            s.subject,
            Subject::Named {
                name: "Counter-Strike 2 at 1.2.3.4:27015".into()
            }
        );
        assert!(s.facts.contains(&Fact::IpAddressLeaves));
    }
}

#[tokio::test]
async fn our_own_launch_is_routine_and_a_claimed_favourite_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    let (class, _) = class_of(&host, &launch(None, JoinSource::Installed))
        .await
        .unwrap();
    assert_eq!(class, IntentClass::Routine);
    // Not stored: claiming it a favourite cannot skip the review.
    let refused = class_of(&host, &launch(Some("1.2.3.4:27015"), JoinSource::Favourite)).await;
    assert!(matches!(refused, Err(IpcResponse::Error { code: 404, .. })));
    let (answer, _) = run(
        &host,
        IpcRequest::FavouriteServerAdd {
            game_id: 4181,
            server: ServerAddress::parse("1.2.3.4:27015").unwrap(),
            label: String::new(),
        },
    )
    .await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    let (class, _) = class_of(&host, &launch(Some("1.2.3.4:27015"), JoinSource::Favourite))
        .await
        .unwrap();
    assert_eq!(class, IntentClass::Routine);
}

#[test]
fn a_server_must_match_its_source_and_the_game_must_launch() {
    let refused = |r: &IpcRequest| crate::validation::validate_request(r).is_err();
    assert!(refused(&launch(
        Some("1.2.3.4:27015"),
        JoinSource::Installed
    )));
    assert!(refused(&launch(None, JoinSource::Invite)));
    assert!(!refused(&launch(None, JoinSource::Installed)));
    assert!(!refused(&launch(Some("1.2.3.4:27015"), JoinSource::Invite)));
    assert!(refused(&IpcRequest::GameLaunch {
        game_id: 0,
        server: None,
        source: JoinSource::Installed,
    }));
}

/// With detection off the Steam library is never read: only played games,
/// minus the hide list.
#[tokio::test]
async fn installed_games_read_no_steam_library_with_detection_off() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::preferences::set_game_detection_enabled(c, &o, false))
        .await
        .unwrap();
    let (answer, class) = run(&host, IpcRequest::InstalledGamesGet).await;
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    assert!(data.as_array().unwrap().is_empty(), "{data}");
    assert_eq!(class, None);
}
