//! Plan E2.5j: each stream's attempt runs off the pump, so a stream whose
//! attempt never resolves holds neither the pump nor another stream.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::*;

fn attempted(o: &Outbox<Fake>, id: &str) -> bool {
    o.deps().store.lock().attempted.iter().any(|a| a == id)
}

async fn until(what: impl Fn() -> bool) -> bool {
    for _ in 0..64 {
        if what() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    false
}

/// A handshake call to Bob that is never answered is in flight when a
/// message to Bob is enqueued: the message is attempted and lands at once,
/// the handshake row still waiting, and the handshake stream is not
/// attempted a second time while its first attempt runs.
#[tokio::test]
async fn a_stream_whose_attempt_never_resolves_does_not_delay_another_stream() {
    let o = outbox();
    let fh = StreamKind::FriendHandshake;
    o.deps().store.lock().hang.insert(fh.stream("bob"));
    let stop = CancellationToken::new();
    let body = async {
        o.enqueue(entry(fh, "bob", "key"), ()).await.unwrap();
        assert!(
            until(|| attempted(&o, "key")).await,
            "the call is in flight"
        );
        advance(&o, 1);
        o.enqueue(entry(StreamKind::Dm, "bob", "msg"), ())
            .await
            .unwrap();
        assert!(
            until(|| state_of(&o, "msg") == Some(SendState::Sent)).await,
            "the message was attempted beside the hung call: {:?}",
            o.deps().store.lock().attempted
        );
        assert_eq!(state_of(&o, "key"), Some(SendState::Queued));
        // Past the handshake row's backoff the pump does not start a second
        // attempt of its stream.
        advance(&o, 60_000);
        o.wake();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let calls = o.deps().store.lock().attempted.clone();
        assert_eq!(calls.iter().filter(|a| *a == "key").count(), 1, "{calls:?}");
        stop.cancel();
    };
    tokio::join!(o.run(stop.clone()), body);
}
