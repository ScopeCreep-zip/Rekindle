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
    let routes = OwnRoutes::with_backoff(
        Arc::clone(fake),
        ready.follower(),
        Duration::from_millis(1),
        Arc::new(|| 1_000),
    );
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
/// identifier-free death line, with its cause.
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
    let veilid = Deaths::during(|| routes.on_dead(&[0x1f2e_3d4c]));
    assert_eq!(veilid.len(), 1, "{veilid:?}");
    assert_identifier_free(&veilid[0]);
    assert!(
        veilid[0].contains("class=General cause=veilid hops=3 age_s=0 node_ready=true"),
        "{}",
        veilid[0]
    );
    settle().await;
    let tested = Deaths::during(|| routes.replace(RouteClass::General));
    assert_eq!(tested.len(), 1, "{tested:?}");
    assert_identifier_free(&tested[0]);
    assert!(
        tested[0].contains("cause=rotated hops=3 age_s=0 node_ready=true"),
        "{}",
        tested[0]
    );
    // A route that is not ours, or a release, is no death.
    settle().await;
    assert!(Deaths::during(|| routes.on_dead(&[77])).is_empty());
    assert!(Deaths::during(|| routes.release(RouteClass::General)).is_empty());
}

fn many(n: u32) -> Vec<Result<(u32, Vec<u8>), AllocError>> {
    (1..=n)
        .map(|id| Ok((id, vec![u8::try_from(id % 251).unwrap()])))
        .collect()
}

/// A budget of `tokens`, its spends counted into `saved`.
fn attach(
    routes: &Arc<OwnRoutes<Arc<Fake>>>,
    tokens: u32,
) -> Arc<Mutex<Vec<crate::budget::RouteBudgetState>>> {
    let saved = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&saved);
    let budget = crate::budget::RouteBudgetState {
        tokens,
        ..crate::budget::RouteBudget::full(0).state()
    };
    routes.attach_budget(
        budget,
        Arc::new(|| 1_000),
        Arc::new(move |b| sink.lock().push(b)),
    );
    saved
}

/// Plan RV1: a forged-signal storm (dead-route reports, rotations) spends
/// a token per new route and stops at the budget: the class stays down
/// (`Allocating`), the pause shows, and no route is drawn past it.
#[tokio::test]
async fn a_signal_storm_spends_the_budget_and_the_class_stays_down() {
    let fake = Arc::new(Fake::new(many(50)));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    let saved = attach(&routes, 3);
    assert_eq!(saved.lock().len(), 1, "the unlock pays for the live route");
    for round in 0..10 {
        match routes.id(RouteClass::General) {
            Some(id) if round % 2 == 0 => routes.on_dead(&[id]),
            Some(_) => routes.replace(RouteClass::General),
            None => break,
        }
        settle().await;
    }
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        3,
        "1 before the unlock + 2 paid"
    );
    assert_eq!(saved.lock().len(), 3);
    assert_eq!(routes.id(RouteClass::General), None);
    assert_eq!(
        *routes.state(RouteClass::General).borrow(),
        RouteState::Allocating
    );
    assert_eq!(
        *routes.paused().borrow(),
        Some(RoutePauseCause::Exhausted),
        "the Host is told"
    );
    assert_eq!(
        routes.deaths(RouteClass::General),
        OwnDeaths { total: 3, young: 3 },
        "each died within a minute of landing: young, counted apart"
    );
}

/// Plan RV1: an unlock whose budget is empty releases the live route it
/// cannot pay for; a relay offer is refused.
#[tokio::test]
async fn an_empty_budget_refuses_and_releases() {
    let fake = Arc::new(Fake::new(many(5)));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    let saved = attach(&routes, 0);
    assert!(saved.lock().is_empty());
    assert_eq!(*fake.released.lock(), vec![1]);
    assert_eq!(*routes.paused().borrow(), Some(RoutePauseCause::Exhausted));
    assert!(!routes.spend_external(), "no relay route past the cap");
    routes.want(RouteClass::General);
    settle().await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1, "nothing drawn");
}

/// Plan RV1 (real-app suite seed): a bucket seeded at 0 refuses an
/// unlock's first allocation: nothing is drawn and the pause shows
/// `Exhausted`, as `REKINDLE_ROUTE_BUDGET_CAPACITY=0` gives a debug build.
#[tokio::test]
async fn a_budget_seeded_empty_refuses_the_first_allocation() {
    let fake = Arc::new(Fake::new(many(5)));
    let (routes, _ready) = owner(&fake, true);
    let saved = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&saved);
    routes.attach_budget(
        crate::budget::RouteBudget::seeded(0, 1_000).state(),
        Arc::new(|| 1_000),
        Arc::new(move |b| sink.lock().push(b)),
    );
    routes.want(RouteClass::General);
    settle().await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0, "nothing drawn");
    assert_eq!(routes.id(RouteClass::General), None);
    assert!(saved.lock().is_empty(), "nothing spent");
    assert_eq!(*routes.paused().borrow(), Some(RoutePauseCause::Exhausted));
}

/// Plan RV1: a stored budget that cannot be read pays for nothing: the live
/// routes are released before they are published, every class stays down
/// with the cause `Unreadable`, no route is drawn and no relay is offered
/// (fail-closed and visible, never a full budget in its place).
#[tokio::test]
async fn an_unreadable_budget_keeps_every_class_down_and_says_why() {
    let fake = Arc::new(Fake::new(many(5)));
    let (routes, _ready) = owner(&fake, true);
    routes.want_all();
    settle().await;
    routes.attach_unreadable_budget();
    assert_eq!(*routes.paused().borrow(), Some(RoutePauseCause::Unreadable));
    assert_eq!(routes.id(RouteClass::General), None);
    assert_eq!(routes.id(RouteClass::Media), None);
    assert_eq!(fake.released.lock().len(), 2);
    assert!(!routes.spend_external());
    routes.want_all();
    settle().await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2, "nothing drawn");
}

/// Plan RV1: a route that dies after the young-route age is not young.
#[tokio::test(start_paused = true)]
async fn a_death_past_the_young_age_is_not_young() {
    let fake = Arc::new(Fake::new(many(5)));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    tokio::time::sleep(crate::budget::YOUNG_ROUTE_AGE).await;
    routes.on_dead(&[1]);
    settle().await;
    assert_eq!(
        routes.deaths(RouteClass::General),
        OwnDeaths { total: 1, young: 0 }
    );
}

/// Plan RV1: no route is drawn on a timer: a live route stays for days.
#[tokio::test(start_paused = true)]
async fn no_allocation_happens_on_a_timer() {
    let fake = Arc::new(Fake::new(many(5)));
    let (routes, _ready) = owner(&fake, true);
    routes.want_all();
    settle().await;
    let calls = fake.calls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(3 * 24 * 60 * 60)).await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), calls);
    assert_eq!(routes.id(RouteClass::General), Some(1));
}

/// Every state `class` passes through from now on, as a follower sees it.
fn record_states(
    routes: &Arc<OwnRoutes<Arc<Fake>>>,
    class: RouteClass,
) -> Arc<Mutex<Vec<RouteState>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut rx = routes.state(class);
    let sink = Arc::clone(&seen);
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let state = rx.borrow_and_update().clone();
            sink.lock().push(state);
        }
    });
    seen
}

fn failing_tests(n: usize) -> Vec<Result<(u32, Vec<u8>), AllocError>> {
    let mut script = vec![Ok((1, b"blob".to_vec()))];
    script.extend((0..n).map(|_| {
        Err(AllocError::TryAgain(
            "allocated route failed to test".into(),
        ))
    }));
    script
}

/// Plan RV1g A3: a route Veilid reported dead is withdrawn once the class
/// has been down past the grace; a failed test records no death (A4).
#[tokio::test(start_paused = true)]
async fn a_veilid_reported_death_withdraws_the_route_after_the_grace() {
    let fake = Arc::new(Fake::new(failing_tests(40)));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    let seen = record_states(&routes, RouteClass::General);
    routes.on_dead(&[1]);
    tokio::time::sleep(
        crate::own_route_down::WITHDRAW_GRACE
            .checked_sub(Duration::from_secs(1))
            .expect("the grace is longer than a second"),
    )
    .await;
    assert!(
        !seen.lock().contains(&RouteState::Withdrawn),
        "withdrawn before the grace"
    );
    tokio::time::sleep(Duration::from_secs(20)).await;
    let withdrawals = seen
        .lock()
        .iter()
        .filter(|s| **s == RouteState::Withdrawn)
        .count();
    assert_eq!(withdrawals, 1, "withdrawn once after the grace");
    assert_eq!(
        routes.deaths(RouteClass::General).total,
        1,
        "failed tests count no death"
    );
    assert!(routes.down().borrow().general.is_some());
}

/// Plan RV1g A3: our own rotation never withdraws the route, however long
/// the replacement keeps failing.
#[tokio::test(start_paused = true)]
async fn a_rotation_never_withdraws_the_route() {
    let fake = Arc::new(Fake::new(failing_tests(40)));
    let (routes, _ready) = owner(&fake, true);
    routes.want(RouteClass::General);
    settle().await;
    let seen = record_states(&routes, RouteClass::General);
    routes.replace(RouteClass::General);
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(
        !seen.lock().contains(&RouteState::Withdrawn),
        "a rotation withdrew the route"
    );
    assert!(routes.down().borrow().general.is_some());
}
