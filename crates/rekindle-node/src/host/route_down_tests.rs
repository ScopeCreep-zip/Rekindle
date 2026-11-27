use super::*;
use rekindle_types::subscription_events::OwnRouteOutage;

fn down(general: Option<(u64, &str)>) -> OwnRouteDownSet {
    OwnRouteDownSet {
        general: general.map(|(since_ms, reason)| OwnRouteOutage {
            since_ms,
            reason: reason.into(),
        }),
        media: None,
    }
}

/// One down event on the transition, nothing while it stays down, one
/// up event when it ends.
#[test]
fn a_class_going_down_and_up_is_told_once_each_way() {
    let up = down(None);
    let out = down(Some((1_000, "allocated route failed to test")));
    let first = changes(&up, &out);
    assert!(matches!(
        first.as_slice(),
        [NetworkEvent::OwnRouteDown {
            class: OwnRouteClass::General,
            since_ms: 1_000,
            ..
        }]
    ));
    assert!(changes(&out, &out).is_empty());
    assert!(matches!(
        changes(&out, &up).as_slice(),
        [NetworkEvent::OwnRouteUp {
            class: OwnRouteClass::General
        }]
    ));
    assert!(changes(&up, &up).is_empty());
}
