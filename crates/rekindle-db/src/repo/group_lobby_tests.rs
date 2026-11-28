//! The lobby fold (plan E7.2 G6) is idempotent: each step replayed changes
//! nothing, and replaying a whole run of entries gives the same rows.

use super::*;
use crate::repo::fixture::{self, OWNER};
use crate::repo::group_dms::{self, GroupDmRow};
use rekindle_types::group_dm::GroupDmRosterMember;

const G: &str = "0123456789abcdef0123456789abcdef";
const L: &str = "1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b";

fn key(n: char) -> String {
    n.to_string().repeat(64)
}

fn conn_with_group() -> Connection {
    let conn = fixture::conn();
    let member = |n: char| GroupDmRosterMember {
        membership_generation: [1; 16],
        identity: key(n),
        profile_record_key: format!("VLD0:{}", "P".repeat(43)),
        display_name: format!("member {n}"),
    };
    group_dms::insert(
        &conn,
        OWNER,
        &GroupDmRow {
            group_id: G.to_string(),
            name: "raid night".to_string(),
            roster: vec![member('a'), member('b'), member('c')],
            epoch: 0,
            created_at: 5,
            origin_peer: None,
            favourite: false,
        },
    )
    .unwrap();
    conn
}

fn server() -> ServerAddress {
    ServerAddress::parse("203.0.113.7:27015").unwrap()
}

/// One lobby entry, as its writer's chain carries it.
enum Entry {
    Open(char),
    Join(char),
    Leave(char),
    Launch(char),
    Close(char),
}

fn fold(conn: &Connection, entry: &Entry) -> bool {
    match entry {
        Entry::Open(w) => open(conn, OWNER, G, L, &key(*w), 4181, 100),
        Entry::Join(w) => join(conn, OWNER, G, L, &key(*w)),
        Entry::Leave(w) => leave(conn, OWNER, G, L, &key(*w)),
        Entry::Launch(w) => launch(conn, OWNER, G, L, &key(*w), &server()),
        Entry::Close(w) => close(conn, OWNER, G, L, &key(*w)),
    }
    .unwrap()
}

fn lobby(state: LobbyState, server: Option<&str>, members: &[char]) -> LobbyRow {
    LobbyRow {
        lobby_id: L.to_string(),
        host_key: key('a'),
        game_id: 4181,
        state,
        server: server.map(str::to_string),
        opened_at: 100,
        members: members.iter().map(|m| key(*m)).collect(),
    }
}

/// Open, join twice, leave: the second join and the replayed leave change
/// nothing; the host never joins its own lobby.
#[test]
fn open_join_join_leave_folds_once() {
    let conn = conn_with_group();
    assert_eq!(get(&conn, OWNER, G, L).unwrap(), None);
    assert!(fold(&conn, &Entry::Open('a')));
    assert!(!fold(&conn, &Entry::Open('b')), "first open wins");
    assert!(fold(&conn, &Entry::Join('b')));
    assert!(!fold(&conn, &Entry::Join('b')), "replayed join");
    assert!(!fold(&conn, &Entry::Join('a')), "the host never joins");
    assert!(fold(&conn, &Entry::Join('c')));
    assert!(fold(&conn, &Entry::Leave('c')));
    assert!(!fold(&conn, &Entry::Leave('c')), "replayed leave");
    assert_eq!(
        get(&conn, OWNER, G, L).unwrap(),
        Some(lobby(LobbyState::Open, None, &['b']))
    );
}

/// Only the host launches (once, from open) and closes (once); the launch
/// sets the server and state, the close keeps the server, and a join after
/// either is refused.
#[test]
fn launch_and_close_come_only_from_the_host_once() {
    let conn = conn_with_group();
    fold(&conn, &Entry::Open('a'));
    fold(&conn, &Entry::Join('b'));
    assert!(!fold(&conn, &Entry::Launch('b')), "not the host");
    assert!(fold(&conn, &Entry::Launch('a')));
    assert!(!fold(&conn, &Entry::Launch('a')), "replayed launch");
    let launched = Some(lobby(
        LobbyState::Launched,
        Some("203.0.113.7:27015"),
        &['b'],
    ));
    assert_eq!(get(&conn, OWNER, G, L).unwrap(), launched);
    assert!(!fold(&conn, &Entry::Join('c')), "joined after the launch");
    assert!(!fold(&conn, &Entry::Close('b')), "not the host");
    assert!(fold(&conn, &Entry::Close('a')));
    assert!(!fold(&conn, &Entry::Close('a')), "replayed close");
    assert!(
        !fold(&conn, &Entry::Launch('a')),
        "launched after the close"
    );
    let closed = Some(lobby(LobbyState::Closed, Some("203.0.113.7:27015"), &['b']));
    assert_eq!(get(&conn, OWNER, G, L).unwrap(), closed);
}

/// A member's join read before the host's open is kept (the fold does not
/// depend on which chain is read first), a host's own early join is
/// dropped by its open, and replaying every entry gives the same rows.
#[test]
fn replaying_the_entries_gives_the_same_rows() {
    let conn = conn_with_group();
    let run = [
        Entry::Join('b'),
        Entry::Join('a'),
        Entry::Open('a'),
        Entry::Join('c'),
        Entry::Leave('c'),
        Entry::Launch('a'),
    ];
    for entry in &run {
        fold(&conn, entry);
    }
    let first = get(&conn, OWNER, G, L).unwrap();
    assert_eq!(
        first,
        Some(lobby(
            LobbyState::Launched,
            Some("203.0.113.7:27015"),
            &['b']
        ))
    );
    for entry in &run {
        fold(&conn, entry);
    }
    assert_eq!(get(&conn, OWNER, G, L).unwrap(), first);
}

/// The lobby rows go with their group, and a lobby needs a held group.
#[test]
fn a_lobby_needs_its_group_and_goes_with_it() {
    let conn = conn_with_group();
    fold(&conn, &Entry::Open('a'));
    fold(&conn, &Entry::Join('b'));
    assert!(group_dms::delete(&conn, OWNER, G).unwrap());
    assert_eq!(get(&conn, OWNER, G, L).unwrap(), None);
    assert!(
        !join(&conn, OWNER, G, L, &key('b')).is_ok_and(|j| j),
        "no group"
    );
    assert!(open(&conn, OWNER, G, L, &key('a'), 4181, 1).is_err());
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM group_lobby_members", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

/// The group shows its open lobby before any newer closed one, and its
/// newest lobby once none is open.
#[test]
fn current_prefers_the_open_lobby_then_the_newest() {
    let conn = conn_with_group();
    assert_eq!(current(&conn, OWNER, G).unwrap(), None);
    let other = "f".repeat(32);
    assert!(open(&conn, OWNER, G, L, &key('a'), 4181, 100).unwrap());
    assert!(open(&conn, OWNER, G, &other, &key('b'), 4181, 200).unwrap());
    assert!(close(&conn, OWNER, G, &other, &key('b')).unwrap());
    assert_eq!(current(&conn, OWNER, G).unwrap().unwrap().lobby_id, L);
    assert!(close(&conn, OWNER, G, L, &key('a')).unwrap());
    assert_eq!(current(&conn, OWNER, G).unwrap().unwrap().lobby_id, other);
}
