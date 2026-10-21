//! One pool call: run it as a task on the pool's scope and wait for it,
//! until logout releases the wait (plan C7.6g).
//!
//! The handover is a oneshot. `send` fails exactly when the waiter is gone,
//! so a result is either claimed by its caller or given to the call's
//! [`Unclaimed`] handler; an open or create can never be lost between the
//! two (it would stay open in Veilid, writer key and watches, with no lease).

use std::future::Future;
use std::sync::Arc;

use futures::future::BoxFuture;
use rekindle_lifecycle::SessionScope;
use tokio_util::sync::CancellationToken;

/// What a call's task does with a result its caller no longer waits for.
pub(super) type Unclaimed<C, T> = fn(C, T) -> BoxFuture<'static, ()>;

/// Spawn `fut` on `scope` and wait for its result, or for `waits` to be
/// cancelled. `released` is the error a released (or refused) wait gets.
/// `ctx` is handed to `unclaimed` with a result nobody claimed.
pub(super) async fn handover<C, T, E, Fut>(
    scope: &Arc<SessionScope>,
    name: &'static str,
    ctx: C,
    fut: Fut,
    unclaimed: Option<Unclaimed<C, T>>,
    waits: &CancellationToken,
    released: E,
) -> Result<T, E>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Clone + Send + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = scope.spawn(name, async move {
        if let Err(Ok(value)) = tx.send(fut.await) {
            if let Some(handle) = unclaimed {
                handle(ctx, value).await;
            }
        }
    });
    if spawned.is_err() {
        return Err(released);
    }
    tokio::select! {
        biased;
        result = rx => result.unwrap_or(Err(released)),
        () = waits.cancelled() => Err(released),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    fn scope() -> Arc<SessionScope> {
        SessionScope::new("test pool", Arc::new(|_| {}))
    }

    fn count(seen: Arc<AtomicU32>, value: u32) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            seen.store(value, Ordering::SeqCst);
        })
    }

    #[tokio::test]
    async fn a_claimed_result_reaches_its_caller() {
        let waits = CancellationToken::new();
        let seen = Arc::new(AtomicU32::new(0));
        let got = handover(
            &scope(),
            "call",
            Arc::clone(&seen),
            async { Ok::<u32, ()>(7) },
            Some(count),
            &waits,
            (),
        )
        .await;
        assert_eq!(got, Ok(7));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            seen.load(Ordering::SeqCst),
            0,
            "a claimed result is not unclaimed"
        );
    }

    #[tokio::test]
    async fn drain_releases_the_wait_and_the_call_runs_on_to_its_handler() {
        let scope = scope();
        let waits = CancellationToken::new();
        let seen = Arc::new(AtomicU32::new(0));
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
        let call = handover(
            &scope,
            "call",
            Arc::clone(&seen),
            async move {
                let _ = finish_rx.await;
                Ok::<u32, ()>(9)
            },
            Some(count),
            &waits,
            (),
        );
        let release = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            waits.cancel();
        };
        let (got, ()) = tokio::join!(call, release);
        assert_eq!(got, Err(()), "the released wait returns at once");
        assert_eq!(seen.load(Ordering::SeqCst), 0, "the call is still running");
        let _ = finish_tx.send(());
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            seen.load(Ordering::SeqCst),
            9,
            "the unclaimed result is handled"
        );
    }

    #[tokio::test]
    async fn a_closed_scope_refuses_the_call() {
        let scope = scope();
        scope.shutdown(Duration::from_millis(10)).await.ok();
        let waits = CancellationToken::new();
        let got = handover(
            &scope,
            "call",
            (),
            async { Ok::<u32, ()>(1) },
            None,
            &waits,
            (),
        )
        .await;
        assert_eq!(got, Err(()));
    }
}
