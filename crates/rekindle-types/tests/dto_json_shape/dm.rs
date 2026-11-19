//! The direct-message DTOs in the golden DTO file (plan E2.1, E2.5, E7.1).

use rekindle_types::dto::dm::{DisappearingTimerView, DmInboxEntry, DmMessage};
use rekindle_types::message::SendState;
use serde_json::Value;

use super::{pk, to};

/// A message, a history-gap row, a timer-change notice row, the timer
/// control's view and an inbox entry.
pub(super) fn samples(add: &mut impl FnMut(&str, Value)) {
    add(
        "dm.message",
        to(DmMessage {
            id: 9,
            message_id: Some("0192d5a8-7f00-7c3a-9b1e-4d2f6a8c0e11".into()),
            sender_id: pk('a'),
            body: "gg".into(),
            timestamp: 1_700_000_000_000,
            is_own: true,
            send_state: SendState::Delivered,
            history_gap_through: None,
            expire_seconds: 0,
            expire_at: None,
            timer_notice_seconds: None,
        }),
    );
    add(
        "dm.historyGap",
        to(DmMessage {
            id: 10,
            message_id: None,
            sender_id: pk('b'),
            body: String::new(),
            timestamp: 1_700_000_000_000,
            is_own: false,
            send_state: SendState::Delivered,
            history_gap_through: Some(4),
            expire_seconds: 0,
            expire_at: None,
            timer_notice_seconds: None,
        }),
    );
    add(
        "dm.timerNotice",
        to(DmMessage {
            id: 11,
            message_id: Some("0192d5a8-7f00-7c3a-9b1e-4d2f6a8c0e12".into()),
            sender_id: pk('b'),
            body: String::new(),
            timestamp: 1_700_000_000_000,
            is_own: false,
            send_state: SendState::Delivered,
            history_gap_through: None,
            expire_seconds: 3600,
            expire_at: None,
            timer_notice_seconds: Some(3600),
        }),
    );
    add("dm.disappearingTimer", to(DisappearingTimerView::of(3600)));
    add(
        "dm.inboxEntry",
        to(DmInboxEntry {
            peer_key: pk('b'),
            sender_key: pk('b'),
            body: "gg".into(),
            timestamp: 1_700_000_000_000,
            is_read: false,
        }),
    );
}
