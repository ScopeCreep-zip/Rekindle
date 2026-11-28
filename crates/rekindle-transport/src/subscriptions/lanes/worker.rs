/// Run `handle` on each item `pop` yields until the stop flag turns true,
/// or its sender is gone: the dispatch loop ended without the flag (its task
/// panicked or was aborted). A gone sender's `changed()` is ready at once on
/// every poll, so a worker that only read the flag re-polled it forever,
/// one busy runtime thread per worker (IC1).
pub async fn work_until_stopped<T, P, PF, H, HF>(
    mut stop: tokio::sync::watch::Receiver<bool>,
    worker: &'static str,
    mut pop: P,
    mut handle: H,
) where
    P: FnMut() -> PF,
    PF: std::future::Future<Output = T>,
    H: FnMut(T) -> HF,
    HF: std::future::Future<Output = ()>,
{
    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    tracing::info!(worker, "ingress worker shutting down");
                    break;
                }
            }
            item = pop() => handle(item).await,
        }
    }
}
