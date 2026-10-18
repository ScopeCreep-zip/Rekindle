use std::collections::VecDeque;

use super::*;

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
}

fn owner(fake: &Arc<Fake>, ready: bool) -> (Arc<OwnRoutes<Arc<Fake>>>, watch::Sender<bool>) {
    let (tx, rx) = watch::channel(ready);
    let routes = OwnRoutes::with_backoff(Arc::clone(fake), rx, Duration::from_millis(1));
    (routes, tx)
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
