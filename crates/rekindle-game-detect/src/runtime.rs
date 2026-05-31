//! 30-second game-detection poll runtime with shutdown + change-only
//! publish semantics.
//!
//! Owns the `GameDetector` + `last_game` change-detection state;
//! delegates all side-effects (UI emit, DHT publish, community fan-out,
//! cache update) to a `GameDetectorPublisher` impl supplied by the
//! Tauri shell. The runtime is `veilid-core`-free and Tauri-free.
//!
//! This is distinct from `scanner::GameDetector::start_scanning` —
//! `start_scanning` broadcasts every scan via a `watch::channel`
//! (downstream filters for changes), whereas this runtime filters
//! changes internally and exposes a one-shot publisher trait. Both
//! coexist; the runtime is the Tauri-shell's preferred entry point.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::scanner::{DetectedGame, GameDetector};

/// Default poll cadence (30 s) — matches the original `game_service.rs`
/// pre-consolidation behavior.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Publisher implemented by the Tauri shell. Called once per detected
/// game-state change. Bundles UI emit + DHT publish + community
/// fan-out + cache update so the runtime never touches `AppState`.
///
/// The plan's port spec used the name `publish_game_status` with a
/// `Result<(), PublishError>` return — implementations swallow errors
/// via tracing today (UI emit / DHT publish / fan-out are all
/// best-effort) so the trait returns `()`. If a future caller needs
/// hard-error handling we'll re-introduce the Result.
#[async_trait]
pub trait GameDetectorPublisher: Send + Sync + 'static {
    /// Called when the detected game-state transitions. `Some(g)` =
    /// game just started or switched; `None` = previously-running game
    /// just stopped (no game now).
    async fn publish_game_status(&self, detected: Option<DetectedGame>);
}

/// Run the poll loop in-place. The caller is responsible for spawning
/// this on whichever task runtime they own (typically
/// `tauri::async_runtime::spawn`). Sending `()` on the matching
/// `mpsc::Sender` (or dropping it) breaks the loop cleanly.
///
/// Generic over the publisher type so the compiler monomorphises the
/// `publish_game_status` call site (matches the plan's port spec
/// `GameDetectorRuntime<P: GameDetectorPublisher>`).
pub async fn run<P: GameDetectorPublisher>(
    mut detector: GameDetector,
    publisher: Arc<P>,
    mut shutdown_rx: mpsc::Receiver<()>,
    poll_interval: Duration,
) {
    tracing::info!("game detection runtime started");
    let mut interval = tokio::time::interval(poll_interval);
    let mut last_game: Option<String> = None;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let detected = detector.scan_once();
                let current_name = detected.as_ref().map(|g| g.game_name.clone());
                if current_name != last_game {
                    publisher.publish_game_status(detected.clone()).await;
                    if let Some(ref name) = current_name {
                        tracing::info!(game = %name, "game detected");
                    } else {
                        tracing::info!("game ended");
                    }
                    last_game = current_name;
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("game detection runtime shutting down");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    struct CapturingPublisher {
        events: Mutex<Vec<Option<DetectedGame>>>,
    }

    #[async_trait]
    impl GameDetectorPublisher for CapturingPublisher {
        async fn publish_game_status(&self, detected: Option<DetectedGame>) {
            self.events.lock().push(detected);
        }
    }

    #[tokio::test]
    async fn shutdown_breaks_the_loop() {
        let detector = GameDetector::new(crate::database::GameDatabase::bundled(), Duration::from_millis(10));
        let publisher = Arc::new(CapturingPublisher {
            events: Mutex::new(Vec::new()),
        });
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);
        let handle = tokio::spawn(run(
            detector,
            publisher.clone(),
            shutdown_rx,
            Duration::from_millis(10),
        ));
        tokio::time::sleep(Duration::from_millis(30)).await;
        shutdown_tx.send(()).await.unwrap();
        handle.await.unwrap();
        // No assertion on events count — sysinfo on macOS doesn't
        // reliably enumerate processes in test env. The point of the
        // test is that shutdown works.
    }
}
