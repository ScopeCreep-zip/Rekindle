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
    "activityAppended",
    "favouriteServerAdded",
    "infoview",
    "skinSignals",
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
        SystemEvent::ActivityAppended { .. } => "activityAppended",
        SystemEvent::FavouriteServerAdded { .. } => "favouriteServerAdded",
        SystemEvent::Infoview(_) => "infoview",
        SystemEvent::SkinSignals { .. } => "skinSignals",
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
                    images: vec!["skin://localhost/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210.png".into()],
                    palette: vec![rekindle_types::dto::skin_library::SkinPaletteColour {
                        role: "accent.primary".into(),
                        rgb: [0x34, 0x91, 0xd8],
                    }],
                    shells: Vec::new(),
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
        ev(
            "activityAppended",
            SystemEvent::ActivityAppended {
                entries: vec![rekindle_types::activity::ActivityListItem {
                    id: 42,
                    at: 1_700_000_000_000,
                    kind: rekindle_types::activity::ActivityKind::StartedGame {
                        friend: pk('a'),
                        game_id: 4_100,
                        server: Some("203.0.113.7:27015".into()),
                    },
                    display_name: Some("Alice".into()),
                    nickname: Some("Al".into()),
                    identity_pin_mismatch: false,
                }],
            },
        ),
        ev(
            "infoview",
            SystemEvent::Infoview(rekindle_types::dto::infoview::InfoviewEvent::Updated {
                window: "main".into(),
                view: rekindle_types::dto::infoview::InfoviewView {
                    subject: rekindle_types::dto::infoview::InfoviewSubject::Community {
                        community: "c1".into(),
                    },
                    sections: vec![rekindle_types::dto::infoview::InfoviewSection::GameServers {
                        servers: vec![rekindle_types::dto::infoview::InfoviewServerRow {
                            label: "Frag Night".into(),
                            address: "203.0.113.7:27015".into(),
                            game_id: "4100".into(),
                            join: rekindle_types::dto::infoview::InfoviewAction::JoinServer {
                                game_id: Some(4_100),
                                server: Some("203.0.113.7:27015".into()),
                                source: rekindle_types::dto::infoview::JoinSource::Community,
                            },
                        }],
                    }],
                },
            }),
        ),
        ev(
            "favouriteServerAdded",
            SystemEvent::FavouriteServerAdded {
                game_id: 4_100,
                server: "203.0.113.7:27015".into(),
                label: "Frag Night".into(),
            },
        ),
        ev(
            "skinSignals",
            SystemEvent::SkinSignals {
                window: "chat-0123456789abcdef".into(),
                set: rekindle_types::dto::skin_signals::SkinSignalSet {
                    focused: true,
                    can: [
                        rekindle_types::dto::skin_action::Capability::Chat,
                        rekindle_types::dto::skin_action::Capability::Profile,
                    ]
                    .into(),
                    cannot: [rekindle_types::dto::skin_action::Capability::Join].into(),
                    user_typing: true,
                    voice_ringing: false,
                    talking: false,
                    disconnected: false,
                    new_notification: true,
                },
            },
        ),
    ]
}
