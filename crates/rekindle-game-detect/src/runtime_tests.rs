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
    let detector = GameDetector::new(
        crate::database::GameDatabase::bundled(),
        Duration::from_millis(10),
        Arc::new(rekindle_utils::SystemClock),
    );
    let publisher = Arc::new(CapturingPublisher {
        events: Mutex::new(Vec::new()),
    });
    let stop = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(run(
        detector,
        publisher.clone(),
        stop.clone(),
        Duration::from_millis(10),
    ));
    tokio::time::sleep(Duration::from_millis(30)).await;
    stop.cancel();
    handle.await.unwrap();
    // No assertion on events count — sysinfo on macOS doesn't
    // reliably enumerate processes in test env. The point of the
    // test is that shutdown works.
}
