//! RV5 H1: a sealed call held past [`direct::CALL_DEADLINE`] is never
//! half done. Its work runs to completion off the reply, and its nonce
//! stays tentative until the store commits, so a retry is answered "retry
//! later" while the first copy is in flight and receipted only once the
//! message is stored, exactly once.

use std::time::Duration;

use rekindle_transport::operations::direct;

use super::direct_tests::{ann, now_ms, requests, sealed_request};

/// The database reads a sealed request's open makes before its nonce is
/// seen: the block list, then the friendship.
const READS_BEFORE_THE_NONCE: usize = 2;

/// A hold on the database thread: `.0` resolves once the thread reaches
/// it, and the thread waits there until `.1` is sent (or dropped).
fn gate(
    db: &rekindle_db::Db,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let (reached, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel::<()>();
    db.call_detached(move |_| {
        let _ = reached.send(());
        let _ = wait.recv();
    })
    .unwrap();
    (started, release)
}

/// Let the call in flight run up to its next await.
async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_sealed_call_held_past_its_deadline_is_stored_once_then_receipted() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate();
    let frame = sealed_request(&bob, owner, [11; 16], now_ms());
    let datagram = rekindle_transport::operations::dm::wire(&frame).unwrap();
    let db = host.ctx.db.current().unwrap();

    let (_, first) = gate(&db);
    let driver = async {
        // The open's reads go through one at a time; the next hold is
        // queued behind each before it runs.
        let mut held = first;
        for _ in 0..READS_BEFORE_THE_NONCE {
            settle().await;
            let (reached, next) = gate(&db);
            held.send(()).unwrap();
            reached.await.unwrap();
            held = next;
        }
        // The nonce is seen; the request's first read after it (its
        // token) waits behind `held` past the deadline.
        settle().await;
        tokio::time::sleep(direct::CALL_DEADLINE + Duration::from_millis(500)).await;
        held
    };
    let (called, held) = tokio::join!(host.receive_direct_call(&datagram, None), driver);
    assert!(
        later(&called),
        "past the deadline our own delay answers retry later, not {called:?}"
    );

    // The first retry goes as the hold lifts, while the first copy may
    // still be in flight: it is told to retry later, or receipted only
    // once the request is stored, never a duplicate's receipt before.
    let mut held = Some(held);
    for attempt in 0.. {
        let release = async {
            settle().await;
            if let Some(held) = held.take() {
                held.send(()).unwrap();
            }
        };
        let (answer, ()) = tokio::join!(host.receive_direct_call(&datagram, None), release);
        match answer {
            direct::Called::Answered(reply) if reply.is_empty() => {
                assert_eq!(requests(&host, owner).await, 1, "receipted once stored");
                break;
            }
            other => {
                assert!(later(&other), "a retry is no refusal: {other:?}");
                assert!(attempt < 50, "never receipted");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    assert_eq!(requests(&host, owner).await, 1, "stored exactly once");
}

/// Whether `called` is our own "retry later".
fn later(called: &direct::Called) -> bool {
    matches!(called, direct::Called::RetryLater)
}
