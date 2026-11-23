//! The group DM view: its keys, its messages by member label, and the
//! Host's retry line while a send goes unanswered (plan E2.6, SR2b).

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rekindle_types::dto::group_dm::GroupDmViewMember;

fn group(id: char, pending: bool) -> GroupDmView {
    GroupDmView {
        group_id: std::iter::repeat_n(id, 32).collect(),
        name: format!("group {id}"),
        members: vec![GroupDmViewMember {
            key: "b".repeat(64),
            label: "bob".into(),
        }],
        epoch: 0,
        pending,
        origin_peer: None,
        favourite: false,
        rename_refused: None,
        invite_refused: None,
    }
}

fn press(view: &mut GroupDmListView, code: KeyCode) -> Option<Action> {
    view.handle_focused_key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn invites_accept_or_decline_and_groups_open_or_leave() {
    let mut view = GroupDmListView::new();
    view.on_command_result(CommandResult::GroupDmsLoaded {
        groups: vec![group('a', true), group('b', false)],
    })
    .unwrap();
    assert!(matches!(
        press(&mut view, KeyCode::Char('a')),
        Some(Action::AcceptGroupInvite { .. })
    ));
    assert!(press(&mut view, KeyCode::Enter).is_none());
    press(&mut view, KeyCode::Down);
    assert!(press(&mut view, KeyCode::Char('a')).is_none());
    assert!(matches!(
        press(&mut view, KeyCode::Enter),
        Some(Action::ShowGroupMessages { .. })
    ));
    assert!(matches!(
        press(&mut view, KeyCode::Char('L')),
        Some(Action::LeaveGroup { .. })
    ));
}

#[test]
fn messages_name_members_by_label_and_close_when_the_group_is_gone() {
    let mut view = GroupDmListView::new();
    let open = group('b', false);
    let id = open.group_id.clone();
    view.on_command_result(CommandResult::GroupDmsLoaded { groups: vec![open] })
        .unwrap();
    view.on_command_result(CommandResult::GroupMessagesLoaded {
        group_id: id.clone(),
        messages: vec![DmMessage {
            id: 1,
            message_id: None,
            sender_id: "b".repeat(64),
            body: "gg".into(),
            timestamp: 0,
            is_own: false,
            send_state: SendState::Delivered,
            history_gap_through: None,
            expire_seconds: 0,
            expire_at: None,
            timer_notice_seconds: None,
        }],
    })
    .unwrap();
    let (_, lines) = view.message_lines();
    assert!(lines[0].to_string().ends_with("bob: gg"));
    view.on_command_result(CommandResult::GroupDmsLoaded { groups: vec![] })
        .unwrap();
    assert!(view.open.is_none());
}

/// Plan E7.1: a message shows its countdown, a timer change its notice
/// (named by the member's label), and an expired message leaves.
#[test]
fn disappearing_shows_the_countdown_and_notice_and_drops_expired() {
    let mut view = GroupDmListView::new();
    let open = group('b', false);
    let id = open.group_id.clone();
    view.on_command_result(CommandResult::GroupDmsLoaded { groups: vec![open] })
        .unwrap();
    let now = rekindle_utils::timestamp_ms_i64();
    view.on_command_result(CommandResult::GroupMessagesLoaded {
        group_id: id.clone(),
        messages: vec![DmMessage {
            id: 1,
            message_id: Some("m1".into()),
            sender_id: "b".repeat(64),
            body: "psst".into(),
            timestamp: now - 1_000,
            is_own: false,
            send_state: SendState::Delivered,
            history_gap_through: None,
            expire_seconds: 300,
            expire_at: Some(now + 299_000),
            timer_notice_seconds: None,
        }],
    })
    .unwrap();
    let (_, lines) = view.message_lines();
    assert!(
        lines[0]
            .to_string()
            .ends_with("bob: psst (disappears in 4m 59s)"),
        "{}",
        lines[0]
    );

    let context = ConversationContext::GroupDm {
        group_id: id.clone(),
    };
    view.on_subscription_event(&SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DisappearingTimerChanged {
            context: context.clone(),
            seconds: 3_600,
            set_by: "b".repeat(64),
            client_id: "n1".into(),
            timestamp: now,
        },
    ))
    .unwrap();
    let (_, lines) = view.message_lines();
    assert_eq!(
        lines[1].to_string(),
        "-- bob set disappearing messages to 1 hour --"
    );

    view.on_subscription_event(&SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::MessagesExpired {
            context,
            client_ids: vec!["m1".into()],
        },
    ))
    .unwrap();
    let (_, lines) = view.message_lines();
    assert_eq!(lines.len(), 1, "only the notice is left");

    // A reopened group shows the stored notice; its expiry removes it.
    view.on_command_result(CommandResult::GroupMessagesLoaded {
        group_id: id,
        messages: vec![notice_row(&"b".repeat(64), 60, "n2", now)],
    })
    .unwrap();
    let (_, lines) = view.message_lines();
    assert_eq!(
        lines[0].to_string(),
        "-- bob set disappearing messages to 1 minute --"
    );
    view.on_subscription_event(&SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::MessagesExpired {
            context: ConversationContext::GroupDm {
                group_id: view.open.as_ref().unwrap().0.clone(),
            },
            client_ids: vec!["n2".into()],
        },
    ))
    .unwrap();
    assert!(view.message_lines().1.is_empty());
}

#[test]
fn an_unanswered_group_send_says_still_trying_until_it_settles() {
    let mut view = GroupDmListView::new();
    let open = group('b', false);
    let id = open.group_id.clone();
    view.on_command_result(CommandResult::GroupDmsLoaded { groups: vec![open] })
        .unwrap();
    view.on_command_result(CommandResult::GroupMessagesLoaded {
        group_id: id.clone(),
        messages: vec![DmMessage {
            id: 1,
            message_id: Some("c1".into()),
            sender_id: "me".into(),
            body: "gg".into(),
            timestamp: 5,
            is_own: true,
            send_state: SendState::Sent,
            history_gap_through: None,
            expire_seconds: 0,
            expire_at: None,
            timer_notice_seconds: None,
        }],
    })
    .unwrap();
    let status = |retry: Option<DeliveryRetry>, state: SendState| {
        SubscriptionEvent::GroupDm(GroupDmEvent::SendStatus {
            group_id: id.clone(),
            client_id: "c1".into(),
            state,
            retry,
        })
    };
    let retry = DeliveryRetry {
        attempts: 4,
        next_attempt_at: 0,
    };
    view.on_subscription_event(&status(Some(retry), SendState::Sent))
        .unwrap();
    let (_, lines) = view.message_lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].to_string().ends_with("you: gg"), "{lines:?}");
    assert!(
        lines[1].to_string().contains("still trying (attempt 4,"),
        "{lines:?}"
    );

    view.on_subscription_event(&status(None, SendState::Delivered))
        .unwrap();
    let (_, lines) = view.message_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
}
