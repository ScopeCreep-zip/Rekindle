use crate::platform::executor::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

const SHORT_TIMEOUT: Duration = Duration::from_millis(50);

#[tokio::test]
async fn returns_at_once_when_already_applied() {
    let notify = Notify::new();
    assert!(wait_until_applied(&notify, SHORT_TIMEOUT, || true).await);
}

#[tokio::test]
async fn a_stale_event_does_not_end_the_wait() {
    let notify = Notify::new();
    notify.notify_one();
    assert!(!wait_until_applied(&notify, SHORT_TIMEOUT, || false).await);
}

#[tokio::test]
async fn ends_when_a_later_event_finds_the_change_applied() {
    let notify = Arc::new(Notify::new());
    let applied = Arc::new(AtomicBool::new(false));
    notify.notify_one();
    let landing = tokio::spawn({
        let notify = Arc::clone(&notify);
        let applied = Arc::clone(&applied);
        async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            applied.store(true, Ordering::SeqCst);
            notify.notify_one();
        }
    });

    let is_applied = || applied.load(Ordering::SeqCst);
    assert!(wait_until_applied(&notify, Duration::from_secs(5), is_applied).await);
    assert!(is_applied(), "returned before the change landed");
    landing.await.unwrap();
}
