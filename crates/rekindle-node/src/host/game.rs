//! The game stage (plan E1.1), both hosts: the detector's runtime
//! (`rekindle_game_detect::run_runtime`) with a publisher that sets our
//! presence's game, which publishes `SelfChanged` and writes profile
//! subkey 4. `GamePresenceSet`/`GamePresenceClear` write the same slot;
//! the detector's next change overrides a manual set.

use async_trait::async_trait;
use rekindle_game_detect::{DetectedGame, GameDatabase, GameDetector, GameDetectorPublisher};
use tokio_util::sync::CancellationToken;

use super::own_presence::{Game, OwnPresence};

struct Publisher {
    presence: OwnPresence,
}

#[async_trait]
impl GameDetectorPublisher for Publisher {
    async fn publish_game_status(&self, detected: Option<DetectedGame>) {
        let game = detected.map(|detected| Game {
            id: detected.game_id,
            name: detected.game_name,
            server_info: detected
                .rich_presence
                .as_ref()
                .and_then(|rp| rp.details.clone()),
            server_address: detected
                .rich_presence
                .as_ref()
                .and_then(rekindle_game_detect::rich_presence::RichPresence::server_address),
            started_at_ms: detected.started_at_epoch_ms,
        });
        if let Err(e) = self.presence.set_game(game).await {
            tracing::warn!(error = %e, "detected game not published");
        }
    }
}

/// Run the detector until `stop`, stamping each game on the Host's `clock`.
pub(crate) async fn run(
    presence: OwnPresence,
    clock: std::sync::Arc<dyn rekindle_utils::Clock>,
    stop: CancellationToken,
) {
    let detector = GameDetector::new(
        GameDatabase::bundled(),
        rekindle_game_detect::DEFAULT_POLL_INTERVAL,
        clock,
    );
    rekindle_game_detect::run_runtime(
        detector,
        std::sync::Arc::new(Publisher { presence }),
        stop,
        rekindle_game_detect::DEFAULT_POLL_INTERVAL,
    )
    .await;
}
