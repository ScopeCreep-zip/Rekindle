//! `NetworkEvent` samples, covering every `RouteAvailability`.

use rekindle_types::subscription_events::{
    NetworkEvent, OwnRecordsPhase, OwnRouteClass, RouteAvailability, RoutePauseCause,
    SubscriptionEvent,
};

use crate::{pk, rk, text, Sample};

pub const NAMES: &[&str] = &[
    "attachmentChanged",
    "strength",
    "localRoutesDied",
    "remoteRoutesDied",
    "valueChanged",
    "ownRecordsChanged",
    "routePaused",
    "routeResumed",
    "routeDeathsAboveBaseline",
    "ownRouteDown",
    "ownRouteUp",
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
    "OwnRouteClass.general",
    "OwnRouteClass.media",
    "RoutePauseCause.exhausted",
    "RoutePauseCause.unreadable",
];

pub fn variant_name(e: &NetworkEvent) -> &'static str {
    match e {
        NetworkEvent::AttachmentChanged { .. } => "attachmentChanged",
        NetworkEvent::Strength { .. } => "strength",
        NetworkEvent::LocalRoutesDied { .. } => "localRoutesDied",
        NetworkEvent::RemoteRoutesDied { .. } => "remoteRoutesDied",
        NetworkEvent::ValueChanged { .. } => "valueChanged",
        NetworkEvent::OwnRecordsChanged { .. } => "ownRecordsChanged",
        NetworkEvent::RoutePaused { .. } => "routePaused",
        NetworkEvent::RouteResumed {} => "routeResumed",
        NetworkEvent::RouteDeathsAboveBaseline { .. } => "routeDeathsAboveBaseline",
        NetworkEvent::OwnRouteDown { .. } => "ownRouteDown",
        NetworkEvent::OwnRouteUp { .. } => "ownRouteUp",
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
        NetworkEvent::RouteDeathsAboveBaseline { class, .. }
        | NetworkEvent::OwnRouteDown { class, .. }
        | NetworkEvent::OwnRouteUp { class } => vec![match class {
            OwnRouteClass::General => "OwnRouteClass.general",
            OwnRouteClass::Media => "OwnRouteClass.media",
        }],
        NetworkEvent::RoutePaused { cause } => vec![match cause {
            RoutePauseCause::Exhausted => "RoutePauseCause.exhausted",
            RoutePauseCause::Unreadable => "RoutePauseCause.unreadable",
        }],
        NetworkEvent::Strength { .. }
        | NetworkEvent::RouteResumed {}
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
            "routePaused.exhausted",
            NetworkEvent::RoutePaused {
                cause: RoutePauseCause::Exhausted,
            },
        ),
        ev(
            "routePaused.unreadable",
            NetworkEvent::RoutePaused {
                cause: RoutePauseCause::Unreadable,
            },
        ),
        ev("routeResumed", NetworkEvent::RouteResumed {}),
        ev(
            "ownRouteDown.general",
            NetworkEvent::OwnRouteDown {
                class: OwnRouteClass::General,
                since_ms: 1_700_000_000_000,
                reason: "allocated route failed to test".into(),
            },
        ),
        ev(
            "ownRouteDown.media",
            NetworkEvent::OwnRouteDown {
                class: OwnRouteClass::Media,
                since_ms: 1_700_000_060_000,
                reason: "allocated route failed to test".into(),
            },
        ),
        ev(
            "ownRouteUp.general",
            NetworkEvent::OwnRouteUp {
                class: OwnRouteClass::General,
            },
        ),
        ev(
            "ownRouteUp.media",
            NetworkEvent::OwnRouteUp {
                class: OwnRouteClass::Media,
            },
        ),
        ev(
            "routeDeathsAboveBaseline.general",
            NetworkEvent::RouteDeathsAboveBaseline {
                class: OwnRouteClass::General,
                deaths: 4,
                young: 2,
                window_secs: 3600,
                baseline: 3,
            },
        ),
        ev(
            "routeDeathsAboveBaseline.media",
            NetworkEvent::RouteDeathsAboveBaseline {
                class: OwnRouteClass::Media,
                deaths: 5,
                young: 0,
                window_secs: 3600,
                baseline: 3,
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
