//! Editing, deleting and removing a selected message in the channel view,
//! and the edited/removed rendering (plan E3.3 slice 9).

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use rekindle_types::display::DecryptedMessageDisplay;
use rekindle_types::dto::community::CommunityMemberView;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::ChannelWatchView;
use crate::action::{Action, CommandResult};
use crate::components::Component;
use crate::views::View;

const ANN: &str = "a22aa22aa22aa22aa22aa22aa22aa22a";
const CAL: &str = "ca11ca11ca11ca11ca11ca11ca11ca11";

fn msg(id: &str, author: &str, body: &str, is_own: bool) -> DecryptedMessageDisplay {
    DecryptedMessageDisplay {
        message_id: id.into(),
        sequence: 0,
        author_pseudonym: author.into(),
        author_display_name: author.into(),
        body: body.into(),
        timestamp: 0,
        reply_to_message_id: None,
        mek_generation: 0,
        is_encrypted: false,
        needs_mek: None,
        automod_blurred: false,
        is_own,
        edited: false,
        removed_by: None,
    }
}

/// A view over "com"/"gen" holding Ann's message `m1` and ours `m2`, with
/// the member names loaded, and the given one selected.
fn view_selecting(index: usize) -> ChannelWatchView {
    let mut view = ChannelWatchView::new("com".into(), "gen".into(), true);
    view.on_command_result(CommandResult::ChannelHistoryLoaded {
        community: "com".into(),
        channel: "gen".into(),
        messages: vec![msg("m1", ANN, "hi", false), msg("m2", CAL, "helo", true)],
    })
    .unwrap();
    let member = |key: &str, name: &str| -> CommunityMemberView {
        serde_json::from_value(serde_json::json!({
            "pseudonymKey": key, "displayName": name, "roleIds": [], "displayRole": "",
            "status": "online", "timeoutUntil": null, "group": {"kind": "online"},
        }))
        .unwrap()
    };
    view.on_command_result(CommandResult::CommunityMembersLoaded {
        community: "com".into(),
        members: vec![member(ANN, "Ann"), member(CAL, "Cal")],
    })
    .unwrap();
    view.message_list.scroll_to_top();
    for _ in 0..index {
        view.message_list.scroll_down();
    }
    view
}

/// The message list as drawn, one string.
fn drawn(view: &mut ChannelWatchView) -> String {
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal
        .draw(|frame| view.message_list.draw(frame, frame.area()).unwrap())
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

#[test]
fn editing_our_selected_message_submits_an_edit_not_a_new_post() {
    let mut view = view_selecting(1);
    view.update(Action::EditSelected).unwrap();
    assert_eq!(
        view.input_box.content(),
        "helo",
        "the input starts from the body"
    );
    view.input_box.set_content("hello");
    let action = view.update(Action::InputSubmit).unwrap();
    assert!(
        matches!(
            action,
            Some(Action::EditChannelMessage { ref channel, ref message_id, ref text })
                if channel == "gen" && message_id == "m2" && text == "hello"
        ),
        "got {action:?}"
    );
}

#[test]
fn another_members_message_is_not_edited() {
    let mut view = view_selecting(0);
    view.update(Action::EditSelected).unwrap();
    assert!(matches!(
        view.input_box.mode(),
        crate::components::input_box::InputMode::Compose
    ));
}

#[test]
fn delete_sends_our_delete_for_our_message_and_a_removal_for_anothers() {
    let mut view = view_selecting(1);
    let key = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('d'));
    assert!(matches!(
        view.message_list.handle_key(key),
        Some(Action::DeleteSelected)
    ));
    assert!(matches!(
        view.update(Action::DeleteSelected).unwrap(),
        Some(Action::DeleteChannelMessage { ref channel, ref message_id })
            if channel == "gen" && message_id == "m2"
    ));
    let mut view = view_selecting(0);
    assert!(matches!(
        view.update(Action::DeleteSelected).unwrap(),
        Some(Action::RemoveChannelMessage { ref community, ref channel, ref message_id })
            if community == "com" && channel == "gen" && message_id == "m1"
    ));
}

#[test]
fn an_edit_shows_edited_and_a_removal_shows_removed_by_the_moderator() {
    let mut view = view_selecting(0);
    view.on_subscription_event(&SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::Edited {
            community: "com".into(),
            channel: "gen".into(),
            message_id: "m2".into(),
            edited_at: 5,
            body: Some("hello".into()),
        },
    ))
    .unwrap();
    view.on_subscription_event(&SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::Deleted {
            community: "com".into(),
            channel: "gen".into(),
            message_id: "m1".into(),
            removed_by: Some(CAL.into()),
        },
    ))
    .unwrap();
    let screen = drawn(&mut view);
    assert!(screen.contains("hello (edited)"), "{screen}");
    assert!(screen.contains("Removed by Cal"), "{screen}");
    assert!(
        !screen.contains("hi "),
        "the removed body is gone: {screen}"
    );
}

#[test]
fn an_authors_delete_takes_the_message_out() {
    let mut view = view_selecting(0);
    view.on_subscription_event(&SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::Deleted {
            community: "com".into(),
            channel: "gen".into(),
            message_id: "m1".into(),
            removed_by: None,
        },
    ))
    .unwrap();
    assert_eq!(view.message_list.len(), 1);
}
