//! `GroupDmEvent` samples, one per variant.

use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{GroupDmEvent, GroupDmMember, SubscriptionEvent};

use crate::{pk, text, Sample};

pub const NAMES: &[&str] = &["invited", "joined", "left", "keyRotated", "sendStatus"];

pub fn variant_name(e: &GroupDmEvent) -> &'static str {
    match e {
        GroupDmEvent::Invited { .. } => "invited",
        GroupDmEvent::Joined { .. } => "joined",
        GroupDmEvent::Left { .. } => "left",
        GroupDmEvent::KeyRotated { .. } => "keyRotated",
        GroupDmEvent::SendStatus { .. } => "sendStatus",
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
            "sendStatus",
            GroupDmEvent::SendStatus {
                group_id: group(),
                client_id: text("0192f0c4-7b3a-7c1e-9a55-1f0e2d3c4b5a"),
                state: SendState::Sent,
            },
        ),
    ]
}
