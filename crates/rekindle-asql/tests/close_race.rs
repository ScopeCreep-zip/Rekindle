//! Calls made after, or queued behind, a successful `close` see the closed
//! connection deterministically (finding AQ1). A separate test binary: these
//! tests hold calls in flight on purpose, which a process-wide call watch
//! would count (that watch's test, `call_watch.rs`, is its own binary too).

use rekindle_asql::{Connection, Error};

/// A waker that reports each wake on `woken` and, if `hold` is set, blocks
/// the waking thread until `release` gets a message. The close's reply wakes
/// it on the background thread, so holding it there pins that thread at the
/// instant the closer could first observe the close (finding AQ1).
struct HoldingWaker {
    woken: crossbeam_channel::Sender<()>,
    release: Option<crossbeam_channel::Receiver<()>>,
}

impl std::task::Wake for HoldingWaker {
    fn wake(self: std::sync::Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &std::sync::Arc<Self>) {
        let _ = self.woken.send(());
        if let Some(release) = &self.release {
            let _ = release.recv();
        }
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(ToString::to_string)
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

/// Once `close` resolves, the queue is already disconnected: a call made at
/// that instant, with the background thread pinned right after it replied, is
/// refused at once instead of queued and discarded unrun (finding AQ1).
#[tokio::test]
async fn a_call_after_close_resolves_is_refused_at_once() {
    use std::future::Future;
    use std::task::{Context, Poll};

    let conn = Connection::open_in_memory().await.unwrap();
    let conn2 = conn.clone();

    let (woken_tx, woken_rx) = crossbeam_channel::unbounded();
    let (release_tx, release_rx) = crossbeam_channel::unbounded();
    let waker = std::task::Waker::from(std::sync::Arc::new(HoldingWaker {
        woken: woken_tx,
        release: Some(release_rx),
    }));
    let mut cx = Context::from_waker(&waker);

    let mut close = std::pin::pin!(conn.close());
    let mut closed = close.as_mut().poll(&mut cx);
    if closed.is_pending() {
        // The background thread is now held inside the reply's wake.
        woken_rx.recv().unwrap();
        closed = close.as_mut().poll(&mut cx);
    }
    assert!(matches!(closed, Poll::Ready(Ok(()))));

    let detached = conn2.call_detached(|_| {});
    let unwrapped = {
        let mut call = std::pin::pin!(conn2.call_unwrap(|_| ()));
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            call.as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
        }))
    };
    release_tx.send(()).unwrap();

    assert!(
        matches!(detached, Err(Error::ConnectionClosed)),
        "a detached call after close must be refused, got {detached:?}"
    );
    let payload = unwrapped.expect_err("call_unwrap after close must panic");
    let text = panic_text(&*payload);
    assert!(
        text.starts_with("database connection should be open"),
        "panicked with {text:?}"
    );
}

/// A call queued behind a `close` that succeeds is discarded unrun, and
/// `call_unwrap` reports that as the closed connection, not as a bug.
#[tokio::test]
async fn a_call_queued_behind_close_sees_the_close() {
    use std::future::Future;
    use std::task::{Context, Poll};

    let conn = Connection::open_in_memory().await.unwrap();
    let conn2 = conn.clone();

    // Hold the background thread so the close and the call queue behind it.
    let (gate_tx, gate_rx) = crossbeam_channel::unbounded::<()>();
    conn.call_detached(move |_| {
        let _ = gate_rx.recv();
    })
    .unwrap();

    let (woken_tx, woken_rx) = crossbeam_channel::unbounded();
    let waker = std::task::Waker::from(std::sync::Arc::new(HoldingWaker {
        woken: woken_tx,
        release: None,
    }));
    let mut cx = Context::from_waker(&waker);

    let mut close = std::pin::pin!(conn.close());
    assert!(close.as_mut().poll(&mut cx).is_pending());
    let mut plain = std::pin::pin!(conn2.call(|c| c.execute("SELECT 1;", [])));
    assert!(plain.as_mut().poll(&mut cx).is_pending());
    let mut call = std::pin::pin!(conn2.call_unwrap(|_| ()));
    assert!(call.as_mut().poll(&mut cx).is_pending());

    gate_tx.send(()).unwrap();
    // The close's reply and the two dropped reply senders each wake us.
    for _ in 0..3 {
        woken_rx.recv().unwrap();
    }

    assert!(matches!(close.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
    assert!(matches!(
        plain.as_mut().poll(&mut cx),
        Poll::Ready(Err(Error::ConnectionClosed))
    ));
    let payload =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call.as_mut().poll(&mut cx)))
            .expect_err("call_unwrap queued behind close must panic");
    let text = panic_text(&*payload);
    assert!(
        text.starts_with("database connection should be open"),
        "panicked with {text:?}"
    );
}
