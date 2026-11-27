use super::{bars_text, local_clock, RouteAlert};
use rekindle_types::subscription_events::{
    NetworkEvent, OwnRouteClass, OwnRouteDownSet, OwnRouteOutage, RoutePauseCause,
};

/// Plan RV1: the pause, with its cause, and the deaths warning, with its
/// young deaths, are folded into the status line; a refill clears the
/// pause.
#[test]
fn the_status_line_shows_the_route_pause_and_the_deaths_warning() {
    let mut alert = RouteAlert::default();
    assert_eq!(alert.text(), None);
    alert.fold(&NetworkEvent::RoutePaused {
        cause: RoutePauseCause::Unreadable,
    });
    assert_eq!(
        alert.text().as_deref(),
        Some("inbound route paused: route budget unreadable")
    );
    alert.fold(&NetworkEvent::RoutePaused {
        cause: RoutePauseCause::Exhausted,
    });
    alert.fold(&NetworkEvent::RouteDeathsAboveBaseline {
        class: OwnRouteClass::General,
        deaths: 5,
        young: 2,
        window_secs: 3600,
        baseline: 3,
    });
    assert_eq!(
        alert.text().as_deref(),
        Some(
            "inbound route paused after repeated failures: possible attack · \
                 general route deaths above baseline: 5 in 60m (2 young)"
        )
    );
    alert.fold(&NetworkEvent::RouteResumed {});
    assert_eq!(
        alert.text().as_deref(),
        Some("general route deaths above baseline: 5 in 60m (2 young)")
    );
    // Plan RV1g: an outage shows from the event and from a snapshot,
    // and clears on the way up.
    // The times are local clock time: the expected text is computed
    // with the same local conversion, so the test holds in any zone.
    alert = RouteAlert::default();
    let now = chrono::Local::now();
    let today_ms = u64::try_from(now.timestamp_millis()).expect("after 1970");
    alert.fold(&NetworkEvent::OwnRouteDown {
        class: OwnRouteClass::General,
        since_ms: today_ms,
        reason: "allocated route failed to test".into(),
    });
    assert_eq!(
        alert.text(),
        Some(format!(
            "Unreachable — retrying since {}",
            now.format("%H:%M")
        ))
    );
    alert.fold(&NetworkEvent::OwnRouteUp {
        class: OwnRouteClass::General,
    });
    assert_eq!(alert.text(), None);
    alert.load_down(&OwnRouteDownSet {
        general: None,
        media: Some(OwnRouteOutage {
            since_ms: 9 * 60_000,
            reason: "r".into(),
        }),
    });
    let then = chrono::DateTime::from_timestamp_millis(9 * 60_000)
        .expect("in range")
        .with_timezone(&chrono::Local);
    assert_eq!(
        alert.text(),
        Some(format!(
            "Calls unreachable — retrying since {}",
            then.format("%Y-%m-%d %H:%M")
        ))
    );
    assert_eq!(local_clock(u64::MAX, now), None);
}

/// The TUI shows the Host's bars as the GUI does (plan E7.2).
#[test]
fn the_bars_light_up_to_the_hosts_strength() {
    assert_eq!(bars_text(3, true), "▁▂▄··");
    assert_eq!(bars_text(0, false), ".....");
    assert_eq!(bars_text(5, false), "|||||");
}
