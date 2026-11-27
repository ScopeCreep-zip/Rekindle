use super::*;

use std::sync::atomic::{AtomicU8, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::deps::PresenceError;

struct Deps {
    status: AtomicU8,
    written: Mutex<Vec<u8>>,
}
#[async_trait]
impl StatusPublisherDeps for Deps {
    fn profile_dht_info(&self) -> Option<String> {
        Some("VLD0:profile".into())
    }
    async fn write_profile_status_subkey(
        &self,
        _key: &str,
        payload: Vec<u8>,
    ) -> Result<(), PresenceError> {
        tokio::task::yield_now().await;
        let (owner, key) = self.presence_key().unwrap();
        let opened = rekindle_secrets::friend_presence::open(&key, &owner, 2, &payload).unwrap();
        self.written.lock().push(opened[0]);
        Ok(())
    }
    fn presence_key(&self) -> Option<(String, rekindle_secrets::friend_presence::PresenceKey)> {
        Some((
            "aa".into(),
            rekindle_secrets::friend_presence::PresenceKey::new(1, [9; 32]),
        ))
    }
    fn current_identity_status(&self) -> Option<UserStatusKind> {
        Some(match self.status.load(Ordering::SeqCst) {
            0 => UserStatusKind::Online,
            1 => UserStatusKind::Away,
            _ => UserStatusKind::Invisible,
        })
    }
    fn now_ms(&self) -> i64 {
        0
    }
}

#[test]
fn a_change_publishes_any_status_and_a_hidden_status_publishes_once() {
    use UserStatusKind::{Away, Invisible, Offline, Online};
    assert_eq!(status_to_publish(Some(Away), None), Some(Away));
    assert_eq!(
        status_to_publish(Some(Offline), Some(Online)),
        Some(Offline)
    );
    assert_eq!(
        status_to_publish(Some(Invisible), Some(Online)),
        Some(Invisible)
    );
    assert_eq!(status_to_publish(Some(Online), Some(Online)), Some(Online));
    for hidden in [Offline, Invisible] {
        for landed in [Offline, Invisible] {
            assert_eq!(status_to_publish(Some(hidden), Some(landed)), None);
        }
        assert_eq!(
            status_to_publish(Some(hidden), None),
            Some(hidden),
            "a failed offline write is retried"
        );
    }
    assert_eq!(status_to_publish(None, None), None, "logged out");
}

/// The bug this loop exists for: a change racing a heartbeat ends with
/// the newer status written last, because each publish reads the
/// status at write time.
#[tokio::test]
async fn a_change_during_a_heartbeat_ends_with_the_newer_status() {
    let deps = Arc::new(Deps {
        status: AtomicU8::new(0),
        written: Mutex::new(Vec::new()),
    });
    let wake = Arc::new(Notify::new());
    let stop = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(run_status_publisher(
        Arc::clone(&deps),
        Arc::clone(&wake),
        stop.clone(),
    ));
    wake.notify_one(); // a publish of Online is under way…
    deps.status.store(1, Ordering::SeqCst); // …when the user goes Away
    wake.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;
    stop.cancel();
    task.await.unwrap();
    let written = deps.written.lock().clone();
    assert_eq!(
        written.last().copied(),
        Some(crate::friend::status_to_wire_byte(UserStatusKind::Away)),
        "written: {written:?}"
    );
}

/// Five minutes of Invisible write exactly one offline STATUS, the
/// shape a real Offline writes, and nothing after it.
#[tokio::test(start_paused = true)]
async fn invisible_writes_offline_once_and_then_nothing() {
    let deps = Arc::new(Deps {
        status: AtomicU8::new(2),
        written: Mutex::new(Vec::new()),
    });
    let wake = Arc::new(Notify::new());
    let stop = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(run_status_publisher(
        Arc::clone(&deps),
        Arc::clone(&wake),
        stop.clone(),
    ));
    wake.notify_one();
    tokio::time::sleep(Duration::from_secs(5 * 60 + 1)).await;
    stop.cancel();
    task.await.unwrap();
    assert_eq!(
        deps.written.lock().clone(),
        vec![crate::friend::status_to_wire_byte(UserStatusKind::Offline)]
    );
}
