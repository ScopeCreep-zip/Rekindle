//! Plan RV2, the fifth evidence source over the ports: a peer's re-sent
//! calls of one receipted frame rotate our General route only when they
//! arrive over it, once per peer cooldown.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_records::port::{
    OwnDeaths, PeerTarget, PortError, RouteBudgetState, RouteClass, RoutePauseCause, RoutePort,
    RouteState, SaveBudget, WallMs,
};
use rekindle_route::liveness::{REPORT_PEER_COOLDOWN, REPORT_ROTATE_AGE};
use tokio::sync::watch;

use super::{owner, Liveness};

/// Our General route, a new id at each replace; replaces counted.
struct OneRoute {
    id: Mutex<u32>,
    replaces: AtomicU32,
    state: watch::Sender<RouteState>,
    paused: watch::Sender<Option<RoutePauseCause>>,
}

impl OneRoute {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            id: Mutex::new(1),
            replaces: AtomicU32::new(0),
            state: watch::channel(RouteState::Allocating).0,
            paused: watch::channel(None).0,
        })
    }

    fn id(&self) -> String {
        format!("r{}", self.id.lock())
    }

    fn replaces(&self) -> u32 {
        self.replaces.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl RoutePort for OneRoute {
    fn import(&self, _blob: &[u8]) -> Result<PeerTarget, PortError> {
        Err(PortError::Routing("no imports here".into()))
    }
    fn on_dead_remote(&self, _dead: &[PeerTarget]) -> Vec<Vec<u8>> {
        Vec::new()
    }
    fn invalidate_after_send_failure(&self, _target: &PeerTarget) {}
    fn own_blob(&self, _class: RouteClass) -> Option<Vec<u8>> {
        None
    }
    fn own_id(&self, class: RouteClass) -> Option<String> {
        (class == RouteClass::General).then(|| self.id())
    }
    fn own_state(&self, _class: RouteClass) -> watch::Receiver<RouteState> {
        self.state.subscribe()
    }
    async fn available_own(&self, _class: RouteClass) -> Vec<u8> {
        Vec::new()
    }
    fn own_age(&self, _class: RouteClass) -> Option<Duration> {
        None
    }
    fn want_own(&self) {}
    fn want_own_class(&self, _class: RouteClass) {}
    fn release_own_class(&self, _class: RouteClass) {}
    fn replace_own_class(&self, class: RouteClass) {
        assert_eq!(class, RouteClass::General);
        self.replaces.fetch_add(1, Ordering::SeqCst);
        *self.id.lock() += 1;
    }
    fn release_retired_own(&self, _class: RouteClass) {}
    fn on_dead_own(&self, _dead: &[String]) {}
    fn renew_own(&self) {}
    fn release_own(&self) {}
    async fn shutdown_own(&self) {}
    fn attach_budget(&self, _state: RouteBudgetState, _wall: WallMs, _save: SaveBudget) {}
    fn attach_unreadable_budget(&self) {}
    fn detach_budget(&self) {}
    fn spend_budget(&self) -> bool {
        true
    }
    fn own_paused(&self) -> watch::Receiver<Option<RoutePauseCause>> {
        self.paused.subscribe()
    }
    fn own_down(&self) -> watch::Receiver<rekindle_types::subscription_events::OwnRouteDownSet> {
        watch::channel(rekindle_types::subscription_events::OwnRouteDownSet::default()).1
    }
    fn own_deaths(&self, _class: RouteClass) -> OwnDeaths {
        OwnDeaths::default()
    }
}

/// Settle the rotation loop's next tick.
async fn tick() {
    tokio::time::sleep(Duration::from_secs(2)).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn re_sent_calls_over_general_rotate_once_per_peer_cooldown() {
    let routes = OneRoute::new();
    let live = Liveness::new(Arc::new(rekindle_utils::OsRandom), routes.clone(), true);
    let (offer, offers) = watch::channel(None);
    tokio::spawn(owner::run_rotations(Arc::clone(&live), offers));
    tick().await;
    tokio::time::sleep(REPORT_ROTATE_AGE).await;

    // Over another route of ours (or none): never counted.
    for route in [Some("media"), None, Some("media")] {
        live.duplicate_call("bo", [1; 32], route);
    }
    tick().await;
    assert_eq!(routes.replaces(), 0, "not over our General route");

    let general = routes.id();
    assert!(live.over_general(Some(&general)));
    live.duplicate_call("bo", [1; 32], Some(&general));
    tick().await;
    assert_eq!(routes.replaces(), 0, "one copy may cross our receipt");
    live.duplicate_call("bo", [1; 32], Some(&general));
    tick().await;
    assert_eq!(routes.replaces(), 1, "two copies: our receipt was lost");
    assert!(!live.over_general(Some(&general)), "the route was replaced");

    tokio::time::sleep(REPORT_ROTATE_AGE).await;
    let general = routes.id();
    for _ in 0..3 {
        live.duplicate_call("bo", [2; 32], Some(&general));
    }
    tick().await;
    assert_eq!(routes.replaces(), 1, "Bo is cooling down");
    for _ in 0..2 {
        live.duplicate_call("cy", [2; 32], Some(&general));
    }
    tick().await;
    assert_eq!(routes.replaces(), 2, "another peer's evidence counts");

    tokio::time::sleep(REPORT_PEER_COOLDOWN).await;
    let general = routes.id();
    for _ in 0..2 {
        live.duplicate_call("bo", [3; 32], Some(&general));
    }
    tick().await;
    assert_eq!(routes.replaces(), 3, "Bo's cooldown ended");
    drop(offer);
}
