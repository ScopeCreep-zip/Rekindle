use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rekindle_types::activity::ActivityKind;

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

/// `o` asks for the page before the oldest entry shown, and that page
/// goes after the list; a reload's first page replaces it.
#[test]
fn older_pages_ask_before_the_oldest_entry_and_go_after_it() {
    let mut view = ActivityView::new();
    view.on_command_result(CommandResult::ActivityLoaded {
        before: None,
        entries: vec![entry(9, "newest"), entry(7, "older")],
    })
    .unwrap();
    let Some(Action::LoadActivity { before }) =
        view.handle_focused_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
    else {
        panic!("o asks for an older page");
    };
    assert_eq!(before, Some(7));
    view.on_command_result(CommandResult::ActivityLoaded {
        before: Some(7),
        entries: vec![entry(4, "oldest")],
    })
    .unwrap();
    let ids: Vec<i64> = view.entries.iter().map(|e| e.id).collect();
    assert_eq!(ids, vec![9, 7, 4]);
    view.on_command_result(CommandResult::ActivityLoaded {
        before: None,
        entries: vec![entry(10, "now")],
    })
    .unwrap();
    assert_eq!(view.entries.len(), 1);
}
