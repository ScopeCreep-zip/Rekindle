//! The DM view's delivery states: stored, live, and the Host's retry
//! line while a send goes unanswered (plan E2.5, SR2).

use super::*;

const THREAD_PEER: &str = "peer";

fn own(id: &str, body: &str, state: SendState) -> DmMessage {
    DmMessage {
        id: 1,
        message_id: Some(id.into()),
        sender_id: "me".into(),
        body: body.into(),
        timestamp: 10,
        is_own: true,
        send_state: state,
        history_gap_through: None,
        expire_seconds: 0,
        expire_at: None,
        timer_notice_seconds: None,
        reply_to_id: None,
    }
}

fn view_with_thread() -> DmInboxView {
    let mut view = DmInboxView::new(true);
    view.on_command_result(CommandResult::DmInboxLoaded {
        threads: vec![DmThreadDisplay {
            peer_key: THREAD_PEER.into(),
            peer_name: "bob".into(),
            last_message_at: 0,
            unread_count: 0,
            messages: vec![],
        }],
    })
    .unwrap();
    view
}

fn lines(view: &DmInboxView) -> Vec<String> {
    view.thread_lines(&view.threads[0])
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn status(state: SendState, retry: Option<DeliveryRetry>) -> SubscriptionEvent {
    SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus {
        conversation_id: THREAD_PEER.into(),
        client_id: "c1".into(),
        state,
        retry,
    })
}

#[test]
fn a_loaded_message_shows_its_stored_state() {
    let mut view = view_with_thread();
    view.on_command_result(CommandResult::DmHistoryLoaded {
        peer_key: THREAD_PEER.into(),
        messages: vec![own("c1", "hi", SendState::Sent)],
    })
    .unwrap();
    assert!(
        lines(&view)[0].ends_with("you: hi (sent)"),
        "{:?}",
        lines(&view)
    );
}

#[test]
fn an_unanswered_send_says_still_trying_until_it_is_delivered() {
    let mut view = view_with_thread();
    view.on_command_result(CommandResult::DmHistoryLoaded {
        peer_key: THREAD_PEER.into(),
        messages: vec![own("c1", "hi", SendState::Queued)],
    })
    .unwrap();
    let retry = DeliveryRetry {
        attempts: 2,
        next_attempt_at: 0,
    };
    view.on_subscription_event(&status(SendState::Sent, Some(retry)))
        .unwrap();
    let shown = lines(&view);
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert!(
        shown[1].contains("Not delivered yet \u{2014} still trying (attempt 2,"),
        "{shown:?}"
    );

    view.on_subscription_event(&status(SendState::Delivered, None))
        .unwrap();
    let shown = lines(&view);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].ends_with("you: hi (delivered)"), "{shown:?}");
}
