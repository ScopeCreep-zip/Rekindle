//! Tell every frontend when one of our route classes goes down and comes
//! back (plan RV1g, F9 A2): `OwnRouteDown` once on the down transition,
//! `OwnRouteUp` once on the way up, never per attempt. The route owner's
//! [`OwnRouteDownSet`] changes only on those transitions; this follows it
//! and turns each change into its events. A window opened later reads the
//! same set from the network status.

use std::sync::Arc;

use rekindle_records::port::RoutePort;
use rekindle_transport::operations::friend::FriendHost;
use rekindle_types::subscription_events::{
    NetworkEvent, OwnRouteClass, OwnRouteDownSet, SubscriptionEvent,
};
use tokio_util::sync::CancellationToken;

/// The events that take `prev` to `now`: a class newly down (or down
/// again since a new instant) is `OwnRouteDown`, a class no longer down is
/// `OwnRouteUp`, and an unchanged class says nothing.
pub(crate) fn changes(prev: &OwnRouteDownSet, now: &OwnRouteDownSet) -> Vec<NetworkEvent> {
    [OwnRouteClass::General, OwnRouteClass::Media]
        .into_iter()
        .filter_map(|class| match (prev.get(class), now.get(class)) {
            (before, Some(outage)) if before != Some(outage) => Some(NetworkEvent::OwnRouteDown {
                class,
                since_ms: outage.since_ms,
                reason: outage.reason.clone(),
            }),
            (Some(_), None) => Some(NetworkEvent::OwnRouteUp { class }),
            _ => None,
        })
        .collect()
}

/// Follow the route owner's down set until `stop`: what is down now, then
/// each transition.
pub(crate) async fn tell_down(
    routes: Arc<dyn RoutePort>,
    friends: super::adapters::friends::HostFriends,
    stop: CancellationToken,
) {
    let mut down = routes.own_down();
    let mut prev = OwnRouteDownSet::default();
    loop {
        let now = down.borrow_and_update().clone();
        for event in changes(&prev, &now) {
            friends.publish(SubscriptionEvent::Network(event));
        }
        prev = now;
        let changed = stop.run_until_cancelled(down.changed()).await;
        if !matches!(changed, Some(Ok(()))) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
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
}
