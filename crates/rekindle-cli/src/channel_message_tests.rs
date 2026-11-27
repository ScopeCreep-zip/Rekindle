//! The change subcommands parse to the GUI's Host requests, and the three
//! states print as the GUI shows them (plan E3.3 slice 9).

use std::collections::HashMap;

use clap::Parser;
use rekindle_ipc::protocol::{ChannelMessageRequest, CommunityRequest, IpcRequest};
use rekindle_types::dto::channel_message::ChannelMessageView;
use rekindle_types::message::SendState;

use super::{change_request, history_text, outcome_line, ChannelTarget, MessageChangeOutcome};
use crate::cli::{Cli, Command};

const MSG: &str = "abcdef1234567890abcdef1234567890";
const CAL_PSEUDONYM: &str = "ca11ca11ca11ca11ca11ca11ca11ca11";
const ANN_PSEUDONYM: &str = "a22aa22aa22aa22aa22aa22aa22aa22a";

fn target() -> ChannelTarget {
    ChannelTarget {
        anchor: "anchor-key".into(),
        channel_id: "chan-id".into(),
        our_pseudonym: CAL_PSEUDONYM.into(),
    }
}

/// The Host request `rekindle channel <args>` sends on `target()`.
fn request_for(args: &[&str]) -> (ChannelMessageRequest, MessageChangeOutcome) {
    let mut argv = vec!["rekindle", "channel"];
    argv.extend_from_slice(args);
    let Some(Command::Channel(cmd)) = Cli::try_parse_from(argv).unwrap().command else {
        panic!("not a channel command");
    };
    match change_request(&cmd, &target()).expect("a change command") {
        (IpcRequest::Community(CommunityRequest::Messages(request)), outcome) => (request, outcome),
        _ => panic!("not a channel message request"),
    }
}

fn names() -> HashMap<String, String> {
    HashMap::from([
        (CAL_PSEUDONYM.to_owned(), "Cal".to_owned()),
        (ANN_PSEUDONYM.to_owned(), "Ann".to_owned()),
    ])
}

#[test]
fn edit_message_parses_to_the_guis_edit_request() {
    let (request, outcome) = request_for(&[
        "edit-message",
        "-c",
        "Fire",
        "-C",
        "general",
        "-i",
        MSG,
        "-m",
        "fixed",
    ]);
    assert!(matches!(
        request,
        ChannelMessageRequest::ChannelMessageEdit { ref channel, ref message_id, ref body }
            if channel == "chan-id" && message_id == MSG && body == "fixed"
    ));
    assert_eq!(outcome, MessageChangeOutcome::Edited);
}

#[test]
fn delete_message_parses_to_the_guis_delete_request() {
    let (request, outcome) =
        request_for(&["delete-message", "-c", "Fire", "-C", "general", "-i", MSG]);
    assert!(matches!(
        request,
        ChannelMessageRequest::ChannelMessageDelete { ref channel, ref message_id }
            if channel == "chan-id" && message_id == MSG
    ));
    assert_eq!(outcome, MessageChangeOutcome::DeletedForMembers);
}

#[test]
fn remove_message_parses_to_the_moderators_admin_delete() {
    let (request, outcome) = request_for(&[
        "remove-message",
        "-c",
        "Fire",
        "-C",
        "general",
        "-i",
        MSG,
        "--reason",
        "spam",
    ]);
    assert!(matches!(
        request,
        ChannelMessageRequest::ChannelMessageAdminDelete { ref community, ref channel, ref message_id, ref reason }
            if community == "anchor-key" && channel == "chan-id" && message_id == MSG
                && reason.as_deref() == Some("spam")
    ));
    assert_eq!(
        outcome,
        MessageChangeOutcome::Removed {
            by: CAL_PSEUDONYM.into()
        }
    );
}

#[test]
fn the_three_outcomes_print_as_the_gui_words_them() {
    let names = names();
    assert_eq!(
        outcome_line(MSG, &MessageChangeOutcome::Edited, &names),
        "Message abcdef12…7890 edited"
    );
    assert_eq!(
        outcome_line(MSG, &MessageChangeOutcome::DeletedForMembers, &names),
        "Message abcdef12…7890 deleted for members"
    );
    assert_eq!(
        outcome_line(
            MSG,
            &MessageChangeOutcome::Removed {
                by: CAL_PSEUDONYM.into()
            },
            &names
        ),
        "Message abcdef12…7890: Removed by Cal"
    );
    // A moderator we hold no name for: the abbreviated key.
    assert_eq!(
        outcome_line(
            MSG,
            &MessageChangeOutcome::Removed {
                by: CAL_PSEUDONYM.into()
            },
            &HashMap::new()
        ),
        "Message abcdef12…7890: Removed by ca11ca11…ca11"
    );
}

fn view(body: &str) -> ChannelMessageView {
    ChannelMessageView {
        admission: None,
        id: 1,
        sender_id: ANN_PSEUDONYM.into(),
        body: body.into(),
        decryption_failed: false,
        undecryptable: None,
        automod_blurred: false,
        timestamp: 0,
        is_own: false,
        server_message_id: Some(MSG.into()),
        reactions: None,
        pinned: None,
        poll: None,
        forwarded_from_author: None,
        forwarded_from_name: None,
        attachment: None,
        flags: 0,
        send_state: SendState::Delivered,
        failure_reason: None,
        edited_at: None,
        removed_by: None,
        history_gap_through: None,
    }
}

#[test]
fn history_shows_edited_and_removed_messages_as_the_gui_does() {
    let names = names();
    assert_eq!(history_text(&view("hello"), &names), "Ann: hello");
    let edited = ChannelMessageView {
        edited_at: Some(5),
        ..view("hello again")
    };
    assert_eq!(history_text(&edited, &names), "Ann: hello again (edited)");
    let removed = ChannelMessageView {
        removed_by: Some(CAL_PSEUDONYM.into()),
        ..view("")
    };
    assert_eq!(history_text(&removed, &names), "Ann: Removed by Cal");
}
