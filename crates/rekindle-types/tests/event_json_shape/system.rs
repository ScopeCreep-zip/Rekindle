//! `SystemEvent` samples.

use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use crate::{channel, community, id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "announcement",
    "raidAlert",
    "raidDetected",
    "autoModAlert",
    "channelLockdown",
    "kicked",
    "bootstrapRequested",
    "bootstrapReceived",
    "syncRequested",
    "syncReceived",
    "auditChainBroken",
];

pub fn variant_name(e: &SystemEvent) -> &'static str {
    match e {
        SystemEvent::Announcement { .. } => "announcement",
        SystemEvent::RaidAlert { .. } => "raidAlert",
        SystemEvent::RaidDetected { .. } => "raidDetected",
        SystemEvent::AutoModAlert { .. } => "autoModAlert",
        SystemEvent::ChannelLockdown { .. } => "channelLockdown",
        SystemEvent::Kicked { .. } => "kicked",
        SystemEvent::BootstrapRequested { .. } => "bootstrapRequested",
        SystemEvent::BootstrapReceived { .. } => "bootstrapReceived",
        SystemEvent::SyncRequested { .. } => "syncRequested",
        SystemEvent::SyncReceived { .. } => "syncReceived",
        SystemEvent::AuditChainBroken { .. } => "auditChainBroken",
    }
}

fn ev(name: &str, e: SystemEvent) -> Sample {
    (format!("system.{name}"), SubscriptionEvent::System(e))
}

pub fn samples() -> Vec<Sample> {
    let mut all = alerts();
    all.extend(sync());
    all
}

fn alerts() -> Vec<Sample> {
    vec![
        ev(
            "announcement",
            SystemEvent::Announcement {
                community: Some(community()),
                body: text("Server maintenance at midnight"),
                timestamp: 1_760_000_000_000,
            },
        ),
        ev(
            "raidAlert",
            SystemEvent::RaidAlert {
                community: community(),
                active: true,
            },
        ),
        ev(
            "raidDetected",
            SystemEvent::RaidDetected {
                community: community(),
                joins_in_window: 25,
                max_joins_per_interval: 10,
                join_interval_seconds: 60,
            },
        ),
        ev(
            "autoModAlert",
            SystemEvent::AutoModAlert {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
                rule_name: text("no-invite-links"),
            },
        ),
        ev(
            "channelLockdown",
            SystemEvent::ChannelLockdown {
                community: community(),
                locked: true,
            },
        ),
        ev(
            "kicked",
            SystemEvent::Kicked {
                community: community(),
            },
        ),
    ]
}

fn sync() -> Vec<Sample> {
    vec![
        ev(
            "bootstrapRequested",
            SystemEvent::BootstrapRequested {
                community: community(),
                joiner_pseudonym: pk('e'),
            },
        ),
        ev(
            "bootstrapReceived",
            SystemEvent::BootstrapReceived {
                community: community(),
            },
        ),
        ev(
            "syncRequested",
            SystemEvent::SyncRequested {
                community: community(),
                channel: channel(),
                since_timestamp: 1_759_999_000_000,
            },
        ),
        ev(
            "syncReceived",
            SystemEvent::SyncReceived {
                community: community(),
                channel: channel(),
                message_count: 12,
            },
        ),
        ev(
            "auditChainBroken",
            SystemEvent::AuditChainBroken { cursor: 99 },
        ),
    ]
}
