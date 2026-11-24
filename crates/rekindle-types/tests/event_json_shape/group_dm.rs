//! `GroupDmEvent` samples, one per variant.

use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{GroupDmEvent, GroupDmMember, SubscriptionEvent};

use crate::{pk, text, Sample};

pub const NAMES: &[&str] = &[
    "invited",
    "joined",
    "left",
    "keyRotated",
    "sendStatus",
    "message",
    "messageSent",
    "historyGap",
    "renamed",
];

pub fn variant_name(e: &GroupDmEvent) -> &'static str {
    match e {
        GroupDmEvent::Invited { .. } => "invited",
        GroupDmEvent::Joined { .. } => "joined",
        GroupDmEvent::Left { .. } => "left",
        GroupDmEvent::KeyRotated { .. } => "keyRotated",
        GroupDmEvent::SendStatus { .. } => "sendStatus",
        GroupDmEvent::Message { .. } => "message",
        GroupDmEvent::MessageSent { .. } => "messageSent",
        GroupDmEvent::HistoryGap { .. } => "historyGap",
        GroupDmEvent::Renamed { .. } => "renamed",
        GroupDmEvent::Lobby { .. } => "lobby",
    }
}

fn ev(name: &str, e: GroupDmEvent) -> Sample {
    (format!("groupDm.{name}"), SubscriptionEvent::GroupDm(e))
}

fn group() -> String {
    "6d".repeat(16)
}

fn member(c: char, name: &str) -> GroupDmMember {
    GroupDmMember {
        public_key: pk(c),
        display_name: text(name),
    }
}

pub fn samples() -> Vec<Sample> {
    vec![
        ev(
            "invited",
            GroupDmEvent::Invited {
                group_id: group(),
                name: text("raid night"),
                inviter: member('a', "Alice"),
                members: vec![
                    member('a', "Alice"),
                    member('b', "Bob"),
                    member('c', "cccc-cccc"),
                ],
            },
        ),
        ev(
            "joined",
            GroupDmEvent::Joined {
                group_id: group(),
                member: member('b', "Bob"),
            },
        ),
        ev(
            "left",
            GroupDmEvent::Left {
                group_id: group(),
                member: member('c', "cccc-cccc"),
            },
        ),
        ev(
            "keyRotated",
            GroupDmEvent::KeyRotated {
                group_id: group(),
                member: member('b', "Bob"),
                epoch: 2,
            },
        ),
        ev(
            "lobby",
            GroupDmEvent::Lobby {
                group_id: group(),
                lobby: Some(rekindle_types::dto::group_lobby::LobbyView {
                    lobby_id: "ab".repeat(16),
                    host_key: "c".repeat(64),
                    game_id: 4181,
                    members: vec!["d".repeat(64)],
                    state: rekindle_types::dto::group_lobby::LobbyStatus::Launched {
                        server: rekindle_types::game_server::ServerAddress::parse(
                            "203.0.113.7:27015",
                        )
                        .unwrap(),
                    },
                    join: rekindle_types::dto::group_lobby::LobbyVerdict::refused(
                        rekindle_types::dto::group_lobby::LobbyRefusal::NotOpen,
                    ),
                    leave: rekindle_types::dto::group_lobby::LobbyVerdict::Allowed,
                    launch: rekindle_types::dto::group_lobby::LobbyVerdict::refused(
                        rekindle_types::dto::group_lobby::LobbyRefusal::NotHost,
                    ),
                    close: rekindle_types::dto::group_lobby::LobbyVerdict::refused(
                        rekindle_types::dto::group_lobby::LobbyRefusal::NotHost,
                    ),
                }),
            },
        ),
        ev(
            "lobby.none",
            GroupDmEvent::Lobby {
                group_id: group(),
                lobby: None,
            },
        ),
        ev(
            "renamed",
            GroupDmEvent::Renamed {
                group_id: group(),
                name: text("raid night II"),
            },
        ),
        ev(
            "sendStatus",
            GroupDmEvent::SendStatus {
                group_id: group(),
                client_id: text("0192f0c4-7b3a-7c1e-9a55-1f0e2d3c4b5a"),
                state: SendState::Sent,
                retry: Some(rekindle_types::message::DeliveryRetry {
                    attempts: 2,
                    next_attempt_at: 1_700_000_003_000,
                }),
            },
        ),
        ev(
            "message",
            GroupDmEvent::Message {
                group_id: group(),
                message_id: text("0192f0c4-7b3a-7c1e-9a55-1f0e2d3c4b5b"),
                sender: member('b', "Bob"),
                body: text("hello group"),
                timestamp: 1_700_000_000_000,
                reply_to: None,
            },
        ),
        ev(
            "messageSent",
            GroupDmEvent::MessageSent {
                group_id: group(),
                reply_to_id: None,
                message: rekindle_types::dto::dm::DmMessage {
                    id: 43,
                    message_id: Some(text("0192f0c4-7b3a-7c1e-9a55-1f0e2d3c4b5c")),
                    sender_id: pk('a'),
                    body: text("gg group"),
                    timestamp: 1_700_000_000_000,
                    is_own: true,
                    send_state: SendState::Queued,
                    history_gap_through: None,
                    expire_seconds: 0,
                    expire_at: None,
                    timer_notice_seconds: None,
                },
            },
        ),
        ev(
            "historyGap",
            GroupDmEvent::HistoryGap {
                group_id: group(),
                member: member('b', "Bob"),
                through_seq: 4,
            },
        ),
    ]
}
