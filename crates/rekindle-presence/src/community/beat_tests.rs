//! The beat engine on a paused clock with a seeded generator (plan E3.4
//! S2).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_codec::member_record::BeatState;
use rekindle_db::test_support::Seeded;
use rekindle_types::presence::SessionStatus;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{beat_delay, BeatEngine, BeatSink, BEAT_DELAY_MAX, BEAT_DELAY_MIN};

struct Sink {
    communities: Mutex<Vec<String>>,
    status: Mutex<SessionStatus>,
    /// Every write: when (since `start`), where, what.
    writes: Mutex<Vec<(Duration, String, BeatState)>>,
    start: Instant,
}

impl Sink {
    fn new(communities: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            communities: Mutex::new(communities.iter().map(|c| (*c).to_owned()).collect()),
            status: Mutex::new(SessionStatus::Online),
            writes: Mutex::new(Vec::new()),
            start: Instant::now(),
        })
    }

    fn writes(&self) -> Vec<(Duration, String, BeatState)> {
        self.writes.lock().clone()
    }

    fn times(&self, community: &str) -> Vec<Duration> {
        (self.writes().into_iter())
            .filter(|(_, c, _)| c == community)
            .map(|(at, _, _)| at)
            .collect()
    }
}

#[async_trait]
impl BeatSink for Sink {
    fn communities(&self) -> Vec<String> {
        self.communities.lock().clone()
    }

    fn status(&self) -> SessionStatus {
        *self.status.lock()
    }

    async fn write_beat(&self, community: &str, state: BeatState) -> Result<(), String> {
        let at = self.start.elapsed();
        self.writes.lock().push((at, community.to_owned(), state));
        Ok(())
    }
}

fn start(
    sink: &Arc<Sink>,
    seed: u64,
) -> (
    Arc<BeatEngine>,
    CancellationToken,
    tokio::task::JoinHandle<()>,
) {
    let engine = Arc::new(BeatEngine::new(sink.clone(), Arc::new(Seeded::new(seed))));
    let stop = CancellationToken::new();
    let task = tokio::spawn({
        let engine = Arc::clone(&engine);
        let stop = stop.clone();
        async move { engine.run(stop).await }
    });
    (engine, stop, task)
}

/// Let the engine run to its next await.
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

/// The delays stay inside [2.5, 7.5] min and fill the range.
#[test]
fn delays_stay_inside_their_bounds_and_spread_across_them() {
    let rng = Seeded::new(7);
    let draws: Vec<Duration> = (0..20_000).map(|_| beat_delay(&rng)).collect();
    assert!(draws
        .iter()
        .all(|d| (BEAT_DELAY_MIN..=BEAT_DELAY_MAX).contains(d)));
    let lo = draws.iter().min().copied().unwrap_or_default();
    let hi = draws.iter().max().copied().unwrap_or_default();
    assert!(lo < BEAT_DELAY_MIN + Duration::from_secs(5), "{lo:?}");
    assert!(
        hi > BEAT_DELAY_MAX.saturating_sub(Duration::from_secs(5)),
        "{hi:?}"
    );
    let mean = draws.iter().sum::<Duration>() / 20_000;
    assert!(
        mean.abs_diff(Duration::from_secs(300)) < Duration::from_secs(5),
        "{mean:?}"
    );
}

/// Scheduled beats keep every gap inside the bounds, and two communities
/// draw their own times.
#[tokio::test(start_paused = true)]
async fn scheduled_beats_keep_their_bounds_per_community() {
    let sink = Sink::new(&["a", "b"]);
    let (_engine, stop, task) = start(&sink, 11);
    tokio::time::sleep(Duration::from_secs(3 * 3600)).await;
    stop.cancel();
    task.await.expect("engine ran");
    let (a, b) = (sink.times("a"), sink.times("b"));
    assert!(a.len() >= 24 && b.len() >= 24, "{} {}", a.len(), b.len());
    for times in [&a, &b] {
        assert!(times[0] >= BEAT_DELAY_MIN && times[0] <= BEAT_DELAY_MAX);
        for gap in times.windows(2).map(|w| w[1].saturating_sub(w[0])) {
            assert!((BEAT_DELAY_MIN..=BEAT_DELAY_MAX).contains(&gap), "{gap:?}");
        }
    }
    assert_ne!(a[..5], b[..5], "two communities beat on one schedule");
    assert!(sink
        .writes()
        .iter()
        .all(|(_, _, s)| *s == BeatState::Online));
}

/// A picked status is written at once, once per community; auto-away (a
/// status change nobody picked) waits for the schedule.
#[tokio::test(start_paused = true)]
async fn a_picked_status_writes_once_at_once_and_idle_waits() {
    let sink = Sink::new(&["a", "b"]);
    let (engine, stop, task) = start(&sink, 3);
    settle().await;
    *sink.status.lock() = SessionStatus::Busy;
    engine.status_changed();
    settle().await;
    let at_once = sink.writes();
    assert_eq!(at_once.len(), 2, "{at_once:?}");
    assert!(at_once
        .iter()
        .all(|(at, _, s)| *at == Duration::ZERO && *s == BeatState::Busy));
    // Auto-away: no trigger, so nothing until a beat falls due.
    *sink.status.lock() = SessionStatus::Away;
    tokio::time::sleep(BEAT_DELAY_MIN.saturating_sub(Duration::from_secs(1))).await;
    assert_eq!(sink.writes().len(), 2);
    tokio::time::sleep(BEAT_DELAY_MAX).await;
    let later = sink.writes();
    assert!(later.len() >= 4);
    assert!(later[2..].iter().all(|(_, _, s)| *s == BeatState::Away));
    stop.cancel();
    task.await.expect("engine ran");
}

/// Invisible writes one Offline beat when picked and nothing after it,
/// until a visible status is picked.
#[tokio::test(start_paused = true)]
async fn invisible_writes_nothing_after_its_offline_beat() {
    let sink = Sink::new(&["a"]);
    let (engine, stop, task) = start(&sink, 5);
    settle().await;
    *sink.status.lock() = SessionStatus::Invisible;
    engine.status_changed();
    settle().await;
    engine.status_changed();
    tokio::time::sleep(Duration::from_secs(2 * 3600)).await;
    assert_eq!(
        sink.writes(),
        vec![(Duration::ZERO, "a".to_owned(), BeatState::Offline)]
    );
    *sink.status.lock() = SessionStatus::Online;
    engine.status_changed();
    settle().await;
    assert_eq!(sink.writes().last().map(|w| w.2), Some(BeatState::Online));
    stop.cancel();
    task.await.expect("engine ran");
}

/// A departure writes `Departed` and ends that community's schedule; the
/// stop ends the engine with no write after it.
#[tokio::test(start_paused = true)]
async fn departure_and_stop_end_the_beats() {
    let sink = Sink::new(&["a", "b"]);
    let (engine, stop, task) = start(&sink, 9);
    settle().await;
    engine.depart("a").await.expect("written");
    tokio::time::sleep(Duration::from_secs(3600)).await;
    let a: Vec<_> = (sink.writes().into_iter()).filter(|w| w.1 == "a").collect();
    assert_eq!(
        a,
        vec![(Duration::ZERO, "a".to_owned(), BeatState::Departed)]
    );
    assert!(sink.times("b").len() >= 8);
    stop.cancel();
    task.await.expect("engine ran");
    let count = sink.writes().len();
    tokio::time::sleep(Duration::from_secs(3600)).await;
    engine.status_changed();
    settle().await;
    assert_eq!(sink.writes().len(), count);
}

/// A community joined between beats is scheduled within the membership
/// check, and one left stops beating.
#[tokio::test(start_paused = true)]
async fn joined_and_left_communities_follow_the_memberships() {
    let sink = Sink::new(&["a"]);
    let (_engine, stop, task) = start(&sink, 13);
    settle().await;
    sink.communities.lock().push("b".to_owned());
    tokio::time::sleep(super::MEMBERSHIP_CHECK + BEAT_DELAY_MAX).await;
    let b = sink.times("b");
    assert!(!b.is_empty());
    sink.communities.lock().retain(|c| c != "a");
    let left_at = sink.start.elapsed();
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert!(sink.times("a").iter().all(|at| *at <= left_at));
    stop.cancel();
    task.await.expect("engine ran");
}

/// The quit writes `Offline` once in every community still beating; after
/// Invisible's own `Offline` beat it writes nothing.
#[tokio::test(start_paused = true)]
async fn the_quit_writes_offline_once() {
    let sink = Sink::new(&["a", "b"]);
    let (engine, stop, task) = start(&sink, 17);
    settle().await;
    stop.cancel();
    task.await.expect("engine ran");
    engine.quit().await;
    engine.quit().await;
    let offline = |w: &(Duration, String, BeatState)| w.2 == BeatState::Offline;
    assert_eq!(sink.writes().iter().filter(|w| offline(w)).count(), 2);

    let sink = Sink::new(&["a"]);
    let (engine, stop, task) = start(&sink, 19);
    settle().await;
    *sink.status.lock() = SessionStatus::Invisible;
    engine.status_changed();
    settle().await;
    stop.cancel();
    task.await.expect("engine ran");
    engine.quit().await;
    assert_eq!(sink.writes().len(), 1);
}
