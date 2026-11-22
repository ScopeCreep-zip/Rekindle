//! The Infoview as text: the Host's sections in its order, rows as the DTO
//! carries them.

use rekindle_types::activity::{ActivityKind, ActivityListItem};
use rekindle_types::dto::infoview::{
    InfoviewAction, InfoviewSection, InfoviewServerRow, InfoviewSubject, InfoviewView, JoinSource,
};
use rekindle_types::friend::NotJoinableReason;

use super::{activity_row, lines, InfoviewLine};

fn entry(id: i64, text: &str) -> ActivityListItem {
    ActivityListItem {
        id,
        at: 0,
        kind: ActivityKind::StatusMessage {
            friend: "ab12ab12ab12ab12ab12ab12ab12ab12".into(),
            text: text.into(),
        },
        display_name: Some("Ann".into()),
        nickname: None,
        identity_pin_mismatch: false,
    }
}

/// A row is the time, the kind's name, the Host's names and key, and the
/// entry's own text.
#[test]
fn a_row_shows_the_kind_and_the_entry_text_as_given() {
    let row = activity_row(&entry(1, "afk"));
    assert!(row.contains("statusMessage  Ann (ab12"), "{row}");
    assert!(row.ends_with("\"afk\""), "{row}");
}

/// Each section is its heading then its rows, in the Host's order; a
/// server's Join carries the Host's verdict.
#[test]
fn sections_are_headings_and_rows_in_the_hosts_order() {
    let view = InfoviewView {
        subject: InfoviewSubject::Community {
            community: "c".into(),
        },
        sections: vec![
            InfoviewSection::Bio {
                text: "hello".into(),
            },
            InfoviewSection::Activity {
                entries: vec![entry(2, "brb")],
                more: Some(InfoviewAction::ActivityMore { before: 2 }),
            },
            InfoviewSection::GameServers {
                servers: vec![InfoviewServerRow {
                    label: "Scrims".into(),
                    address: "198.51.100.2:27015".into(),
                    game_id: "4181".into(),
                    join: InfoviewAction::JoinServer {
                        game_id: Some(4181),
                        server: Some("198.51.100.2:27015".into()),
                        source: JoinSource::Community,
                        joinable: false,
                        joinable_reason: Some(NotJoinableReason::CannotLaunch),
                    },
                }],
            },
        ],
    };
    let got = lines(&view);
    assert_eq!(got[0], InfoviewLine::Heading("Bio".into()));
    assert_eq!(got[1], InfoviewLine::Row("hello".into()));
    assert_eq!(got[2], InfoviewLine::Heading("Activity (1)".into()));
    assert!(matches!(&got[3], InfoviewLine::Row(r) if r.ends_with("\"brb\"")));
    assert_eq!(got[4], InfoviewLine::Row("[View more]".into()));
    assert_eq!(got[5], InfoviewLine::Heading("Servers (1)".into()));
    assert_eq!(
        got[6],
        InfoviewLine::Row(
            "Scrims  198.51.100.2:27015  game 4181  [Join: Rekindle cannot start this game on a server]"
                .into()
        )
    );
    assert_eq!(got.len(), 7);
}
