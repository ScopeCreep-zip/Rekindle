//! `GameLaunch`'s class follows its `JoinSource` (lead decision (50)).
//! Only the selection and validation run: no game is ever launched here.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::game_server::{JoinSource, ServerAddress};
use rekindle_types::intent::{
    acknowledgement, effective_class, Fact, IntentClass, IntentKind, IrreversibleReason,
    ProofRequired, Recipient, RecipientKind, Reversibility, Subject,
};

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
        assert_eq!(
            s.facts,
            vec![Fact::IpAddressLeaves {
                to: Recipient::GameServer {
                    game: "Counter-Strike 2".into(),
                    address: "1.2.3.4:27015".into(),
                }
            }],
            "the review names the server our IP address reaches"
        );
        assert_eq!(
            s.reversibility,
            Reversibility::NeverBecause(IrreversibleReason::DisclosureCannotBeRecalled {
                to: RecipientKind::GameServer
            })
        );
    }
}

#[tokio::test]
async fn a_link_review_keeps_the_site_wording() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    let request = IpcRequest::LinkOpen {
        url: "https://example.org/page".into(),
    };
    let s = super::super::select(&host.ctx, &request, IntentKind::OpenExternalUrl)
        .await
        .unwrap();
    assert_eq!(
        s.facts,
        vec![Fact::IpAddressLeaves {
            to: Recipient::Site {
                host: "example.org".into()
            }
        }]
    );
    assert_eq!(
        s.reversibility,
        Reversibility::NeverBecause(IrreversibleReason::DisclosureCannotBeRecalled {
            to: RecipientKind::Site
        })
    );
}

/// A string literal of the webview's fact wording: its raw text and its
/// fixed fragments (a template literal's text outside `${…}`).
struct Literal {
    raw: String,
    fragments: Vec<String>,
}

/// Read one literal opened by `quote`, pushing it (and any literal nested in
/// a `${…}`) to `out`.
fn read_literal(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    quote: char,
    out: &mut Vec<Literal>,
) {
    let (mut raw, mut fragment, mut fragments) = (String::new(), String::new(), Vec::new());
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    raw.push(next);
                    fragment.push(next);
                }
            }
            c if c == quote => break,
            '$' if quote == '`' && chars.peek() == Some(&'{') => {
                chars.next();
                fragments.push(std::mem::take(&mut fragment));
                raw.push_str("${");
                let mut depth = 1;
                while let Some(e) = chars.next() {
                    match e {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        '"' | '\'' | '`' => read_literal(chars, e, out),
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                }
                raw.push('}');
            }
            c => {
                raw.push(c);
                fragment.push(c);
            }
        }
    }
    fragments.push(fragment);
    out.push(Literal { raw, fragments });
}

/// Every string a fact line is built from, in the webview's wording.
fn fact_line_literals() -> Vec<Literal> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/utils/intent_text.ts"
    );
    let ts = std::fs::read_to_string(path).unwrap();
    let (_, body) = ts.split_once("export function factText").unwrap();
    let (body, _) = body.split_once("\n}\n").unwrap();
    let mut out = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, '"' | '\'' | '`') {
            read_literal(&mut chars, c, &mut out);
        }
    }
    out.retain(|l| l.raw.len() > 3);
    out
}

/// A literal's fixed fragments of three or more words, lower-cased, with
/// the whitespace and punctuation at either end dropped.
fn wording(literal: &Literal) -> impl Iterator<Item = String> + '_ {
    literal
        .fragments
        .iter()
        .map(|f| {
            f.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|f| f.split_whitespace().count() >= 3)
}

#[tokio::test]
async fn the_acknowledgement_is_its_own_statement_not_a_fact_line() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    let (class, s) = class_of(
        &host,
        &launch(Some("1.2.3.4:27015"), JoinSource::FriendPresence),
    )
    .await
    .unwrap();
    assert_eq!(class.proof(), ProofRequired::Acknowledge);
    let ack = acknowledgement(&s.facts, &s.escalations);
    assert_eq!(ack, "I understand this server will see my IP address.");
    let lines = fact_line_literals();
    assert!(lines
        .iter()
        .any(|l| l.raw.contains("will see your IP address")));
    let fragments: Vec<String> = lines.iter().flat_map(wording).collect();
    assert!(
        fragments.iter().any(|f| f == "will see your ip address"),
        "the IP fact line's fixed text is read: {fragments:?}"
    );
    let lower = ack.to_lowercase();
    for line in &lines {
        assert!(
            !line.raw.contains(ack) && !ack.contains(line.raw.as_str()),
            "the acknowledgement copies the fact line {:?}",
            line.raw
        );
    }
    for fragment in &fragments {
        assert!(
            !lower.contains(fragment.as_str()),
            "the acknowledgement copies the fact wording {fragment:?}"
        );
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

/// The installed games' ids, read through the Host.
async fn installed(host: &crate::host::Host) -> Vec<u64> {
    let (answer, class) = run(host, IpcRequest::InstalledGamesGet).await;
    assert_eq!(class, None);
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    data.as_array()
        .unwrap()
        .iter()
        .map(|game| game["gameId"].as_u64().unwrap())
        .collect()
}

/// With detection off the Steam library is never read: only played games,
/// minus the hide list. The Host's own root holds a Steam library with
/// Counter-Strike 2's Steam app, whose games it lists while detection is
/// on.
#[tokio::test]
async fn installed_games_read_no_steam_library_with_detection_off() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let app_id = rekindle_game_detect::GameDatabase::shared()
        .lookup_by_id(4181)
        .and_then(|entry| entry.steam_app_id)
        .unwrap();
    let apps = &host.root().apps;
    assert!(apps.home.starts_with(dir.path()), "a fixture root");
    let library = rekindle_game_detect::installed::SteamLayout::CURRENT
        .roots(&apps.home, apps.program_files_x86.as_deref())
        .remove(0)
        .join("steamapps");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join(format!("appmanifest_{app_id}.acf")),
        format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n}}\n"),
    )
    .unwrap();
    // Every game under that Steam app (Counter-Strike 2 and its predecessor).
    let shared: Vec<u64> = rekindle_game_detect::GameDatabase::shared()
        .ids_with_steam_app_id(app_id)
        .into_iter()
        .map(u64::from)
        .collect();
    assert!(shared.contains(&4181), "{shared:?}");
    assert_eq!(installed(&host).await, shared, "detection on");
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::preferences::set_game_detection_enabled(c, &o, false))
        .await
        .unwrap();
    assert!(installed(&host).await.is_empty(), "detection off");
}
