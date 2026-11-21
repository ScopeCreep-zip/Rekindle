//! `NetworkEvent` samples, covering every `RouteAvailability`.

use rekindle_types::subscription_events::{
    NetworkEvent, OwnRecordsPhase, RouteAvailability, SubscriptionEvent,
};

use crate::{pk, rk, text, Sample};

pub const NAMES: &[&str] = &[
    "attachmentChanged",
    "strength",
    "localRoutesDied",
    "remoteRoutesDied",
    "valueChanged",
    "ownRecordsChanged",
];

pub const NESTED: &[&str] = &[
    "RouteAvailability.idle",
    "RouteAvailability.allocating",
    "RouteAvailability.available",
    "RouteAvailability.unavailable",
    "RouteAvailability.failed",
    "OwnRecordsPhase.pending",
    "OwnRecordsPhase.held",
    "OwnRecordsPhase.failed",
];

pub fn variant_name(e: &NetworkEvent) -> &'static str {
    match e {
        NetworkEvent::AttachmentChanged { .. } => "attachmentChanged",
        NetworkEvent::Strength { .. } => "strength",
        NetworkEvent::LocalRoutesDied { .. } => "localRoutesDied",
        NetworkEvent::RemoteRoutesDied { .. } => "remoteRoutesDied",
        NetworkEvent::ValueChanged { .. } => "valueChanged",
        NetworkEvent::OwnRecordsChanged { .. } => "ownRecordsChanged",
    }
}

fn route_name(route: RouteAvailability) -> &'static str {
    match route {
        RouteAvailability::Idle => "RouteAvailability.idle",
        RouteAvailability::Allocating => "RouteAvailability.allocating",
        RouteAvailability::Available => "RouteAvailability.available",
        RouteAvailability::Unavailable => "RouteAvailability.unavailable",
        RouteAvailability::Failed => "RouteAvailability.failed",
    }
}

pub fn nested_names(e: &NetworkEvent) -> Vec<&'static str> {
    match e {
        NetworkEvent::AttachmentChanged { media_route, .. } => vec![route_name(*media_route)],
        NetworkEvent::OwnRecordsChanged { phase, .. } => vec![match phase {
            OwnRecordsPhase::Pending => "OwnRecordsPhase.pending",
            OwnRecordsPhase::Held => "OwnRecordsPhase.held",
            OwnRecordsPhase::Failed => "OwnRecordsPhase.failed",
        }],
        NetworkEvent::Strength { .. }
        | NetworkEvent::LocalRoutesDied { .. }
        | NetworkEvent::RemoteRoutesDied { .. }
        | NetworkEvent::ValueChanged { .. } => vec![],
    }
}

fn ev(name: &str, e: NetworkEvent) -> Sample {
    (format!("network.{name}"), SubscriptionEvent::Network(e))
}

fn attachment(
    suffix: &str,
    state: &str,
    attached: bool,
    has_route: bool,
    media_route: RouteAvailability,
) -> Sample {
    ev(
        &format!("attachmentChanged.{suffix}"),
        NetworkEvent::AttachmentChanged {
            attachment_state: text(state),
            is_attached: attached,
            public_internet_ready: attached,
            has_route,
            media_route,
            // Attached here means unlocked and not detached: the Host
            // writes, with or without a route ("failed": no route, DHT up).
            can_write: attached,
        },
    )
}

pub fn samples() -> Vec<Sample> {
    vec![
        attachment("idle", "detached", false, false, RouteAvailability::Idle),
        attachment(
            "allocating",
            "attaching",
            false,
            false,
            RouteAvailability::Allocating,
        ),
        attachment(
            "available",
            "attached_good",
            true,
            true,
            RouteAvailability::Available,
        ),
        attachment(
            "unavailable",
            "attached_weak",
            true,
            true,
            RouteAvailability::Unavailable,
        ),
        attachment(
            "failed",
            "attached_weak",
            true,
            false,
            RouteAvailability::Failed,
        ),
        ev("strength", NetworkEvent::Strength { bars: 3 }),
        ev(
            "ownRecordsChanged.pending",
            NetworkEvent::OwnRecordsChanged {
                phase: OwnRecordsPhase::Pending,
                error: None,
            },
        ),
        ev(
            "ownRecordsChanged.held",
            NetworkEvent::OwnRecordsChanged {
                phase: OwnRecordsPhase::Held,
                error: None,
            },
        ),
        ev(
            "ownRecordsChanged.failed",
            NetworkEvent::OwnRecordsChanged {
                phase: OwnRecordsPhase::Failed,
                error: Some("no record pool".into()),
            },
        ),
        ev(
            "localRoutesDied",
            NetworkEvent::LocalRoutesDied { count: 2 },
        ),
        ev(
            "remoteRoutesDied",
            NetworkEvent::RemoteRoutesDied {
                peer_keys: vec![pk('b'), pk('c')],
            },
        ),
        ev(
            "valueChanged",
            NetworkEvent::ValueChanged {
                record_key: rk('v'),
                changed_subkeys: vec![0, 5],
            },
        ),
    ]
}
