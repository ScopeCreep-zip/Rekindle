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
#[path = "call_tests.rs"]
mod tests;
