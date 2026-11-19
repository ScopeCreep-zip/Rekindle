//! The game stage (plan E1.1), both hosts: the detector's runtime
//! (`rekindle_game_detect::run_runtime`) with a publisher that sets our
//! presence's game, which publishes `SelfChanged` and writes profile
//! subkey 4. `GamePresenceSet`/`GamePresenceClear` write the same slot;
//! the detector's next change overrides a manual set.
//!
//! The publisher is also the one writer of `game_sessions` (plan E7.2):
//! each detected session is recorded when it ends, and the one still open
//! when the stage stops (lock, quit) is recorded then. A set by hand is
//! not a detected session and is not recorded. Each recorded session
//! changes our totals, which go to friends while sharing is on
//! (`operations::friend::game_totals`, which reads the setting).
//!
//! Detection is split from sharing (plan E7.2, prod research batch 7):
//! the stage runs only while `game_detection_enabled` is on
//! (`game_stage::GameControl` starts and stops it), and the detected game
//! becomes our shown game only while `share_current_game` is on and the
//! game is not on the hide list; otherwise it is recorded locally only.

use async_trait::async_trait;
use rekindle_game_detect::{DetectedGame, GameDatabase, GameDetector, GameDetectorPublisher};
use tokio_util::sync::CancellationToken;

use super::own_presence::{Game, OwnPresence};

pub(crate) struct Publisher {
    presence: OwnPresence,
    sessions: SessionLog,
    /// What the detector last saw, and whether we showed it.
    detected: tokio::sync::Mutex<Detected>,
}

#[derive(Default)]
struct Detected {
    game: Option<DetectedGame>,
    /// Our shown game came from the detector (so stopping or hiding
    /// clears it; a game set by hand is left).
    shown: bool,
}

/// Told when a recorded session changed our totals.
#[async_trait]
pub(crate) trait TotalsChanged: Send + Sync {
    async fn totals_changed(&self);
}

/// The Host's friends: our totals go to each accepted friend, only while
/// sharing is on (plan E7.2; the transport reads the setting).
#[async_trait]
impl TotalsChanged for super::adapters::friends::HostFriends {
    async fn totals_changed(&self) {
        let reached =
            rekindle_transport::operations::friend::game_totals::send_to_friends(self).await;
        tracing::debug!(reached, "game totals sent after a session");
    }
}

/// The detected session in progress and where finished ones are written.
pub(crate) struct SessionLog {
    db: std::sync::Arc<rekindle_db::DbHandle>,
    owner: String,
    clock: std::sync::Arc<dyn rekindle_utils::Clock>,
    /// Told after each session is written.
    changed: std::sync::Arc<dyn TotalsChanged>,
    /// The game id and start (Unix ms) of the session in progress.
    current: tokio::sync::Mutex<Option<(u32, i64)>>,
}

impl SessionLog {
    pub(crate) fn new(
        db: std::sync::Arc<rekindle_db::DbHandle>,
        owner: String,
        clock: std::sync::Arc<dyn rekindle_utils::Clock>,
        changed: std::sync::Arc<dyn TotalsChanged>,
    ) -> Self {
        Self {
            db,
            owner,
            clock,
            changed,
            current: tokio::sync::Mutex::new(None),
        }
    }

    /// The detector now sees `detected`: a different game or a new start
    /// ends the session in progress, which is recorded.
    pub(crate) async fn observe(&self, detected: Option<&DetectedGame>) {
        let next = detected.filter(|game| game.game_id != 0).map(|game| {
            let started = i64::try_from(game.started_at_epoch_ms).unwrap_or(i64::MAX);
            (game.game_id, started)
        });
        let mut current = self.current.lock().await;
        if *current == next {
            return;
        }
        let ended = std::mem::replace(&mut *current, next);
        drop(current);
        if let Some(session) = ended {
            self.record(session).await;
        }
    }

    /// The stage stops: the session in progress ends now.
    pub(crate) async fn finish(&self) {
        let ended = self.current.lock().await.take();
        if let Some(session) = ended {
            self.record(session).await;
        }
    }

    async fn record(&self, (game_id, started_at): (u32, i64)) {
        let ended_at = self.clock.now_ms_i64().max(started_at);
        let db = match self.db.current() {
            Ok(db) => db,
            Err(e) => {
                tracing::warn!(error = %e, game_id, "game session not recorded");
                return;
            }
        };
        let owner = self.owner.clone();
        let written = db
            .call(move |conn| {
                rekindle_db::repo::game_sessions::record(
                    conn, &owner, game_id, started_at, ended_at,
                )
            })
            .await;
        match written {
            Ok(()) => self.changed.totals_changed().await,
            Err(e) => tracing::warn!(error = %e, game_id, "game session not recorded"),
        }
    }
}

impl Publisher {
    pub(crate) fn new(presence: OwnPresence, sessions: SessionLog) -> Self {
        Self {
            presence,
            sessions,
            detected: tokio::sync::Mutex::new(Detected::default()),
        }
    }

    /// `detected` as friends may see it: `None` while sharing the current
    /// game is off, for a hidden game, or when the settings are unreadable
    /// (nothing is shown that was not allowed).
    async fn shareable(&self, detected: Option<&DetectedGame>) -> Option<Game> {
        let detected = detected?;
        let (owner, game_id) = (self.sessions.owner.clone(), detected.game_id);
        let db = self.sessions.db.current().ok()?;
        let allowed = db
            .call(move |conn| {
                let share = rekindle_db::repo::preferences::load(conn, &owner)?.share_current_game;
                Ok(share && !rekindle_db::repo::hidden_games::is_hidden(conn, &owner, game_id)?)
            })
            .await
            .map_err(|e| tracing::warn!(error = %e, "game sharing settings unreadable"))
            .unwrap_or(false);
        allowed.then(|| shown_game(detected))
    }

    /// Show what the detector holds as the settings now allow: after a
    /// detected change, or when sharing or the hide list changed. A game
    /// set by hand is left unless the detector's game replaces it.
    async fn apply(&self, detected: &mut Detected) {
        let game = self.shareable(detected.game.as_ref()).await;
        if game.is_none() && !detected.shown {
            return;
        }
        detected.shown = game.is_some();
        if let Err(e) = self.presence.set_game(game).await {
            tracing::warn!(error = %e, "detected game not published");
        }
    }

    /// The sharing setting or the hide list changed.
    pub(crate) async fn reapply(&self) {
        let mut detected = self.detected.lock().await;
        self.apply(&mut detected).await;
    }

    /// Detection was turned off (its task has ended): the game it showed
    /// is cleared.
    pub(crate) async fn withdraw(&self) {
        let mut detected = self.detected.lock().await;
        detected.game = None;
        self.apply(&mut detected).await;
    }
}

fn shown_game(detected: &DetectedGame) -> Game {
    Game {
        id: detected.game_id,
        name: detected.game_name.clone(),
        server_info: detected
            .rich_presence
            .as_ref()
            .and_then(|rp| rp.details.clone()),
        server_address: detected
            .rich_presence
            .as_ref()
            .and_then(rekindle_game_detect::rich_presence::RichPresence::server_address),
        started_at_ms: detected.started_at_epoch_ms,
    }
}

#[async_trait]
impl GameDetectorPublisher for Publisher {
    async fn publish_game_status(&self, detected: Option<DetectedGame>) {
        self.sessions.observe(detected.as_ref()).await;
        let mut held = self.detected.lock().await;
        held.game = detected;
        self.apply(&mut held).await;
    }
}

/// Run the detector until `stop`, stamping each game on the Host's
/// clock, then record the session still open. It scans at the identity's
/// `game_scan_interval_secs`, the one interval setting (plan E7.2: the
/// runtime's own 30 s constant drifted from it).
pub(crate) async fn run(publisher: std::sync::Arc<Publisher>, stop: CancellationToken) {
    let interval = publisher.sessions.scan_interval().await;
    let detector = GameDetector::new(
        GameDatabase::bundled(),
        interval,
        std::sync::Arc::clone(&publisher.sessions.clock),
    );
    rekindle_game_detect::run_runtime(detector, std::sync::Arc::clone(&publisher), stop, interval)
        .await;
    publisher.sessions.finish().await;
}

impl SessionLog {
    /// The identity's scan interval; its stored default when unreadable.
    async fn scan_interval(&self) -> std::time::Duration {
        let owner = self.owner.clone();
        let secs = match self.db.current() {
            Ok(db) => db
                .call(move |conn| rekindle_db::repo::preferences::load(conn, &owner))
                .await
                .map(|prefs| prefs.game_scan_interval_secs)
                .map_err(|e| tracing::warn!(error = %e, "game detection: preferences unreadable"))
                .ok(),
            Err(_) => None,
        }
        .unwrap_or_else(|| {
            rekindle_types::preferences::Preferences::default().game_scan_interval_secs
        });
        std::time::Duration::from_secs(u64::from(secs))
    }
}

/// `game_sessions` totals as the profile shows them: named and given
/// their icon key from the bundled database.
pub(crate) fn total_views(
    totals: &[rekindle_db::repo::game_sessions::GameTotal],
) -> Vec<rekindle_types::dto::presence::GameTotalView> {
    totals
        .iter()
        .map(|total| {
            let entry = GameDatabase::shared().lookup_by_id(total.game_id);
            rekindle_types::dto::presence::GameTotalView {
                game_id: total.game_id,
                game_name: entry.map_or_else(
                    || format!("Game {}", total.game_id),
                    |entry| entry.name.clone(),
                ),
                icon_key: entry.and_then(rekindle_game_detect::database::GameEntry::icon_key),
                total_secs: total.total_secs,
                last_played: total.last_played,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "game_tests.rs"]
mod tests;
