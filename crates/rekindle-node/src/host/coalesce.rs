//! The coalescing timer the Host's view workers share (the Info View's and
//! the Servers view's): a burst of shown events arms one deadline, and the
//! view is composed and pushed once when it passes.

use tokio::time::Instant;

/// Wait until `due`, or forever while no push is due (the `select!` arm is
/// also guarded by `due.is_some()`).
pub(crate) async fn sleep_until(due: Option<Instant>) {
    if let Some(due) = due {
        tokio::time::sleep_until(due).await;
    }
}
