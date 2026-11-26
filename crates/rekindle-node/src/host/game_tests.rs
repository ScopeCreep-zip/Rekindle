//! The game publisher's session log (plan E7.2): what `game_sessions`
//! holds after the detector's changes and the stage's stop.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rekindle_game_detect::DetectedGame;
use rekindle_utils::Clock;

use super::{SessionLog, TotalsChanged};

const OWNER: &str = "game-owner";

struct Manual(AtomicU64);

impl Clock for Manual {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    fn monotonic(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

fn detected(game_id: u32, started_at_epoch_ms: u64) -> DetectedGame {
    DetectedGame {
        game_id,
        game_name: format!("game {game_id}"),
        process_name: "game.exe".into(),
        started_at_epoch_ms,
        rich_presence: None,
    }
}

/// How many times the totals were said to have changed.
#[derive(Default)]
struct Changes(AtomicU64);

#[async_trait::async_trait]
impl TotalsChanged for Changes {
    async fn totals_changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Changes {
    fn count(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

async fn log(
    dir: &std::path::Path,
    clock: Arc<Manual>,
) -> (SessionLog, rekindle_db::Db, Arc<Changes>) {
    let db = rekindle_db::open(&dir.join("g.db"), &[3; 32]).unwrap();
    db.call(|c| {
        rekindle_db::repo::identity::insert(c, OWNER, "me", 0, "aaaaaaaaaaaaaaaaaaaaaaaaaa")
    })
    .await
    .unwrap();
    let handle = Arc::new(rekindle_db::DbHandle::default());
    handle.set(db.clone());
    let changes = Arc::new(Changes::default());
    let changed: Arc<dyn TotalsChanged> = Arc::clone(&changes) as _;
    (
        SessionLog::new(handle, OWNER.into(), clock, changed),
        db,
        changes,
    )
}

async fn totals(db: &rekindle_db::Db) -> Vec<(u32, u64, i64)> {
    db.call(|c| rekindle_db::repo::game_sessions::totals(c, OWNER))
        .await
        .unwrap()
        .into_iter()
        .map(|t| (t.game_id, t.total_secs, t.last_played))
        .collect()
}

#[tokio::test]
async fn each_detected_session_is_recorded_when_it_ends_and_on_stop() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(Manual(AtomicU64::new(1_000_000)));
    let (sessions, db, changes) = log(dir.path(), Arc::clone(&clock)).await;

    sessions.observe(Some(&detected(4181, 1_000_000))).await;
    clock.0.store(1_060_000, Ordering::SeqCst);
    // The same session seen again ends nothing.
    sessions.observe(Some(&detected(4181, 1_000_000))).await;
    assert!(totals(&db).await.is_empty());
    assert_eq!(changes.count(), 0, "nothing recorded, nothing to send");

    // Another game ends the first: recorded, and the totals go to friends
    // (plan E7.2; the transport sends only while sharing is on).
    sessions.observe(Some(&detected(4102, 1_060_000))).await;
    assert_eq!(totals(&db).await, vec![(4181, 60, 1_060_000)]);
    assert_eq!(changes.count(), 1);

    clock.0.store(1_090_000, Ordering::SeqCst);
    sessions.observe(None).await;
    clock.0.store(2_000_000, Ordering::SeqCst);
    sessions.observe(Some(&detected(4181, 2_000_000))).await;
    clock.0.store(2_030_000, Ordering::SeqCst);
    // The stage stops (lock, quit): the open session ends now.
    sessions.finish().await;
    assert_eq!(
        totals(&db).await,
        vec![(4181, 90, 2_030_000), (4102, 30, 1_090_000)]
    );
    assert_eq!(changes.count(), 3, "one per recorded session");
}

#[tokio::test]
async fn a_game_with_no_database_id_is_not_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(Manual(AtomicU64::new(5_000)));
    let (sessions, db, changes) = log(dir.path(), clock).await;
    sessions.observe(Some(&detected(0, 1_000))).await;
    sessions.finish().await;
    assert!(totals(&db).await.is_empty());
    assert_eq!(changes.count(), 0);
}

#[test]
fn totals_are_named_and_given_their_icon_from_the_database() {
    let views = super::total_views(&[
        rekindle_db::repo::game_sessions::GameTotal {
            game_id: 4181,
            total_secs: 90,
            last_played: 7,
        },
        rekindle_db::repo::game_sessions::GameTotal {
            game_id: 1,
            total_secs: 5,
            last_played: 3,
        },
    ]);
    assert_eq!(views[0].game_name, "Counter-Strike 2");
    assert_eq!(views[0].icon_key.as_deref(), Some("xfire/8507"));
    assert_eq!((views[0].total_secs, views[0].last_played), (90, 7));
    assert_eq!(views[1].game_name, "Game 1");
    assert_eq!(views[1].icon_key, None);
}
