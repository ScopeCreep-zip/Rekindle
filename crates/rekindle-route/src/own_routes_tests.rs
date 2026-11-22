use std::collections::VecDeque;

use super::*;
use crate::ordered_watch::OrderedWatch;

/// Scripted allocation results per call; records every release.
struct Fake {
    script: Mutex<VecDeque<Result<(u32, Vec<u8>), AllocError>>>,
    released: Mutex<Vec<u32>>,
    calls: std::sync::atomic::AtomicU32,
    /// Holds each allocation until a permit arrives (for the race test).
    gate: Option<tokio::sync::Semaphore>,
}

impl Fake {
    fn new(script: Vec<Result<(u32, Vec<u8>), AllocError>>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            released: Mutex::new(Vec::new()),
            calls: std::sync::atomic::AtomicU32::new(0),
            gate: None,
        }
    }
}

#[async_trait]
impl RouteAllocator for Arc<Fake> {
    type Id = u32;
    async fn allocate(&self, _class: RouteClass) -> Result<(u32, Vec<u8>), AllocError> {
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.script
            .lock()
            .pop_front()
            .unwrap_or(Err(AllocError::Fatal("script empty".into())))
    }
    fn release(&self, id: &u32) {
        self.released.lock().push(*id);
    }
    fn hop_count(&self, _class: RouteClass) -> Option<usize> {
        Some(3)
    }
}

fn owner(fake: &Arc<Fake>, ready: bool) -> (Arc<OwnRoutes<Arc<Fake>>>, Arc<OrderedWatch<bool>>) {
    let ready = Arc::new(OrderedWatch::new(ready));
    let routes =
        OwnRoutes::with_backoff(Arc::clone(fake), ready.follower(), Duration::from_millis(1));
    (routes, ready)
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(40)).await;
}

#[tokio::test]
async fn try_again_is_retried_until_a_route_is_live() {
    let fake = Arc::new(Fake::new(vec![
        Err(AllocError::TryAgain("no peer info".into())),
        Err(AllocError::TryAgain("too few nodes".into())),
        Ok((7, b"blob".to_vec())),
    ]));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 3);
    assert_eq!(routes.blob(RouteClass::General), Some(b"blob".to_vec()));
    assert_eq!(
        *routes.state(RouteClass::General).borrow(),
        RouteState::Available {
            blob: b"blob".to_vec()
        }
    );
    assert!(fake.released.lock().is_empty());
}

#[tokio::test]
async fn nothing_is_allocated_before_the_network_is_ready() {
    let fake = Arc::new(Fake::new(vec![Ok((1, b"a".to_vec()))]));
    let (routes, ready) = owner(&fake, false);
    routes.want(RouteClass::General);
    settle().await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    ready.send_replace(true);
    settle().await;
    assert_eq!(routes.id(RouteClass::General), Some(1));
}

#[tokio::test]
async fn a_dead_route_is_forgotten_not_released_and_replaced() {
    let fake = Arc::new(Fake::new(vec![
        Ok((1, b"a".to_vec())),
        Ok((2, b"b".to_vec())),
    ]));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    routes.on_dead(&[1]);
    settle().await;
    assert_eq!(routes.id(RouteClass::General), Some(2));
    assert!(
        fake.released.lock().is_empty(),
        "a dead route is never released"
    );
}

#[tokio::test]
async fn a_fatal_error_is_a_visible_failure_and_not_retried() {
    let fake = Arc::new(Fake::new(vec![Err(AllocError::Fatal("bad spec".into()))]));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::Media);
    settle().await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *routes.state(RouteClass::Media).borrow(),
        RouteState::Failed {
            reason: "bad spec".into()
        }
    );
}

#[tokio::test]
async fn renew_releases_the_live_route_and_allocates_a_fresh_one() {
    let fake = Arc::new(Fake::new(vec![
        Ok((1, b"a".to_vec())),
        Ok((2, b"b".to_vec())),
        Ok((3, b"c".to_vec())),
    ]));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    routes.renew();
    settle().await;
    assert_eq!(
        *fake.released.lock(),
        vec![1],
        "the session's route is released"
    );
    assert_ne!(
        routes.id(RouteClass::General),
        Some(1),
        "the next login gets a fresh route"
    );
}

#[tokio::test]
async fn a_route_landing_after_release_is_released() {
    let mut fake = Fake::new(vec![Ok((9, b"late".to_vec()))]);
    fake.gate = Some(tokio::sync::Semaphore::new(0));
    let fake = Arc::new(fake);
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    routes.release_all(); // logout while the allocation is in flight
    fake.gate.as_ref().unwrap().add_permits(1);
    settle().await;
    assert_eq!(*fake.released.lock(), vec![9]);
    assert_eq!(routes.id(RouteClass::General), None);
}

#[tokio::test]
async fn wanting_twice_allocates_once() {
    let mut fake = Fake::new(vec![Ok((1, b"a".to_vec())), Ok((2, b"b".to_vec()))]);
    fake.gate = Some(tokio::sync::Semaphore::new(0));
    let fake = Arc::new(fake);
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    routes.want(RouteClass::General);
    settle().await;
    fake.gate.as_ref().unwrap().add_permits(2);
    settle().await;
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        1,
        "single-flight per class"
    );
    assert!(fake.released.lock().is_empty());
}

#[tokio::test]
async fn sending_is_wanted_alone_and_released_alone() {
    let fake = Arc::new(Fake::new(vec![
        Ok((1, b"g".to_vec())),
        Ok((2, b"m".to_vec())),
        Ok((3, b"s".to_vec())),
    ]));
    let (routes, _ready) = owner(&fake, true);
    routes.want_all();
    settle().await;
    assert_eq!(
        routes.id(RouteClass::Sending),
        None,
        "node-ready wants General and Media only"
    );
    routes.want(RouteClass::Sending);
    settle().await;
    assert_eq!(routes.id(RouteClass::Sending), Some(3));
    routes.on_dead(&[3]);
    settle().await;
    assert!(
        fake.released.lock().is_empty(),
        "a dead route is forgotten, not released"
    );
    routes.release(RouteClass::Sending);
    assert_eq!(
        *routes.state(RouteClass::Sending).borrow(),
        RouteState::Idle
    );
    assert_eq!(
        routes.id(RouteClass::General),
        Some(1),
        "the other classes stay"
    );
}

/// Lead decision (60): the `Sending` route keeps a warm standby that a
/// death promotes at once, with no allocation between, and a new standby
/// is allocated behind it. Only `Sending` keeps one, and a release frees it.
#[tokio::test]
async fn a_sending_death_promotes_the_standby_at_once() {
    let fake = Arc::new(Fake::new(vec![
        Ok((1, b"g".to_vec())),
        Ok((3, b"s".to_vec())),
        Ok((4, b"standby".to_vec())),
        Ok((5, b"next".to_vec())),
    ]));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        1,
        "General keeps no standby"
    );
    routes.want(RouteClass::Sending);
    settle().await;
    assert_eq!(routes.id(RouteClass::Sending), Some(3));
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        3,
        "the standby is allocated"
    );
    routes.on_dead(&[3]);
    assert_eq!(routes.id(RouteClass::Sending), Some(4), "promoted at once");
    assert_eq!(
        *routes.state(RouteClass::Sending).borrow(),
        RouteState::Available {
            blob: b"standby".to_vec()
        }
    );
    settle().await;
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        4,
        "a new standby behind it"
    );
    assert_eq!(routes.id(RouteClass::Sending), Some(4));
    routes.release(RouteClass::Sending);
    assert_eq!(
        *fake.released.lock(),
        vec![4, 5],
        "live and standby released"
    );
}

/// Node shutdown with an allocation in flight (live A/B round 3 R3-6):
/// the allocation is awaited, never cut, since Veilid logs ERROR for a
/// dropped `new_custom_private_route` (0.5.8 `veilid_api/api.rs:812-842`,
/// `logging/duration_recorder.rs:84-90`); its route is released when it
/// lands, and only then does `shutdown` return, before the network stops.
#[tokio::test]
async fn shutdown_awaits_an_allocation_in_flight_and_releases_its_route() {
    let mut fake = Fake::new(vec![Ok((9, b"late".to_vec()))]);
    fake.gate = Some(tokio::sync::Semaphore::new(0));
    let fake = Arc::new(fake);
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    let stopping = {
        let routes = Arc::clone(&routes);
        tokio::spawn(async move { routes.shutdown().await })
    };
    settle().await;
    assert!(!stopping.is_finished(), "shutdown waits for the allocation");
    fake.gate.as_ref().unwrap().add_permits(1);
    stopping.await.expect("shutdown ends");
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        1,
        "the allocation ran to its end"
    );
    assert_eq!(*fake.released.lock(), vec![9]);
    assert_eq!(routes.blob(RouteClass::General), None);
}

/// r7 design §4.6: each own-route death path emits exactly one
/// identifier-free death line, with its cause and self-test history.
#[tokio::test]
async fn each_death_path_logs_one_identifier_free_line() {
    use crate::route_death::tests::{assert_identifier_free, Deaths};
    let fake = Arc::new(Fake::new(vec![
        Ok((0x1f2e_3d4c, b"one".to_vec())),
        Ok((0x5b6a_7988, b"two".to_vec())),
        Ok((9, b"three".to_vec())),
    ]));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    routes.self_tested(RouteClass::General, true);
    routes.self_tested(RouteClass::General, false);
    let veilid = Deaths::during(|| routes.on_dead(&[0x1f2e_3d4c]));
    assert_eq!(veilid.len(), 1, "{veilid:?}");
    assert_identifier_free(&veilid[0]);
    assert!(veilid[0].contains("class=General cause=veilid hops=3 age_s=0 failures=1 passes=1 since_ok_s=0 node_ready=true"), "{}", veilid[0]);
    settle().await;
    // The tally started over for the replacement.
    routes.self_tested(RouteClass::General, false);
    let tested = Deaths::during(|| routes.replace(RouteClass::General));
    assert_eq!(tested.len(), 1, "{tested:?}");
    assert_identifier_free(&tested[0]);
    assert!(
        tested[0].contains("cause=self_test hops=3 age_s=0 failures=1 passes=0 since_ok_s=never"),
        "{}",
        tested[0]
    );
    // A route that is not ours, or a release, is no death.
    settle().await;
    assert!(Deaths::during(|| routes.on_dead(&[77])).is_empty());
    assert!(Deaths::during(|| routes.release(RouteClass::General)).is_empty());
}
