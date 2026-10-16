//! `NotificationEvent` samples.

use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use crate::{channel, community, id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "messageReceived",
    "systemAlert",
    "updateAvailable",
    "sessionResetRequested",
    "callIncoming",
];

pub fn variant_name(e: &NotificationEvent) -> &'static str {
    match e {
        NotificationEvent::MessageReceived { .. } => "messageReceived",
        NotificationEvent::SystemAlert { .. } => "systemAlert",
        NotificationEvent::UpdateAvailable { .. } => "updateAvailable",
        NotificationEvent::SessionResetRequested { .. } => "sessionResetRequested",
        NotificationEvent::CallIncoming { .. } => "callIncoming",
    }
}

fn ev(name: &str, e: NotificationEvent) -> Sample {
    (
        format!("notification.{name}"),
        SubscriptionEvent::Notification(e),
    )
}

pub fn samples() -> Vec<Sample> {
    vec![
        ev(
            "messageReceived",
            NotificationEvent::MessageReceived {
                title: text("#general"),
                body: text("Alice: hello channel"),
                community_id: community(),
                channel_id: channel(),
                sound_ref: Some(text("ping")),
            },
        ),
        ev(
            "systemAlert",
            NotificationEvent::SystemAlert {
                title: text("Network"),
                body: text("Reconnected"),
            },
        ),
        ev(
            "updateAvailable",
            NotificationEvent::UpdateAvailable {
                version: text("1.2.3"),
            },
        ),
        ev(
            "sessionResetRequested",
            NotificationEvent::SessionResetRequested {
                peer_public_key: pk('b'),
                peer_display_name: text("Bob"),
                safety_number: text("deadbeef"),
            },
        ),
        ev(
            "callIncoming",
            NotificationEvent::CallIncoming {
                call_id: id("call_", '9'),
                from: pk('b'),
                display_name: text("Bob"),
                kind: text("video"),
                expires_at_ms: 1_760_000_030_000,
                is_group: false,
            },
        ),
    ]
}
