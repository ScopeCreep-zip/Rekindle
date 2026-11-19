//! `SystemEvent` samples.

use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use crate::{channel, community, id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "announcement",
    "raidAlert",
    "raidDetected",
    "autoModAlert",
    "channelLockdown",
    "bootstrapRequested",
    "bootstrapReceived",
    "syncRequested",
    "syncReceived",
    "auditChainBroken",
    "skinChanged",
    "skinTrialChanged",
];

pub fn variant_name(e: &SystemEvent) -> &'static str {
    match e {
        SystemEvent::Announcement { .. } => "announcement",
        SystemEvent::RaidAlert { .. } => "raidAlert",
        SystemEvent::RaidDetected { .. } => "raidDetected",
        SystemEvent::AutoModAlert { .. } => "autoModAlert",
        SystemEvent::ChannelLockdown { .. } => "channelLockdown",
        SystemEvent::BootstrapRequested { .. } => "bootstrapRequested",
        SystemEvent::BootstrapReceived { .. } => "bootstrapReceived",
        SystemEvent::AuditChainBroken { .. } => "auditChainBroken",
        SystemEvent::SkinChanged { .. } => "skinChanged",
        SystemEvent::SkinTrialChanged { .. } => "skinTrialChanged",
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
            "auditChainBroken",
            SystemEvent::AuditChainBroken { cursor: 99 },
        ),
        ev(
            "skinChanged",
            SystemEvent::SkinChanged {
                skin: rekindle_types::dto::skin_library::SkinApplied {
                    id: "builtin:symbiosis".into(),
                    base: rekindle_types::dto::skin::SkinBase::Dark,
                    tokens_href: "skin://localhost/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/tokens.css".into(),
                    regions: vec!["frame".into()],
                    palette: vec![rekindle_types::dto::skin_library::SkinPaletteColour {
                        role: "accent.primary".into(),
                        rgb: [0x34, 0x91, 0xd8],
                    }],
                },
            },
        ),
        ev(
            "skinTrialChanged",
            SystemEvent::SkinTrialChanged {
                trial: Some(rekindle_types::dto::skin_library::SkinTrial {
                    id: "pkg:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                    name: "Carbon Fiber".into(),
                    previous: "builtin:symbiosis".into(),
                    reverts_at_ms: 1_700_000_015_000,
                }),
            },
        ),
    ]
}
