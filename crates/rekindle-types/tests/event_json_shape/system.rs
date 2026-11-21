//! `SystemEvent` samples.

use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use crate::{channel, community, id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "raidDetected",
    "autoModAlert",
    "bootstrapRequested",
    "bootstrapReceived",
    "auditChainBroken",
    "skinChanged",
    "skinTrialChanged",
    "quickWipeChanged",
];

pub fn variant_name(e: &SystemEvent) -> &'static str {
    match e {
        SystemEvent::RaidDetected { .. } => "raidDetected",
        SystemEvent::AutoModAlert { .. } => "autoModAlert",
        SystemEvent::BootstrapRequested { .. } => "bootstrapRequested",
        SystemEvent::BootstrapReceived { .. } => "bootstrapReceived",
        SystemEvent::AuditChainBroken { .. } => "auditChainBroken",
        SystemEvent::SkinChanged { .. } => "skinChanged",
        SystemEvent::SkinTrialChanged { .. } => "skinTrialChanged",
        SystemEvent::QuickWipeChanged { .. } => "quickWipeChanged",
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
        ev(
            "quickWipeChanged",
            SystemEvent::QuickWipeChanged {
                armed: true,
                shortcut: Some("CommandOrControl+Shift+F12".into()),
            },
        ),
    ]
}
