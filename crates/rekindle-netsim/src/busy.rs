//! Database work the paused clock can see (plan E2.B S8, findings F4/F7).
//!
//! A Host's database runs on a `rekindle-asql` thread, outside tokio, so a
//! Host awaiting a call looks idle: tokio would auto-advance the paused
//! clock to the next timer while the thread works, and the pump would
//! deliver before the Host's answer is in. The harness installs an asql
//! call watch (`rekindle_db::watch_calls`) that takes a [`db_call`] for each
//! call. A call queued from a simulation's runtime thread counts in that
//! simulation's [`Busy`]; while any is in
//! flight one blocking task waits for them, and tokio does not auto-advance
//! a current-thread runtime while a blocking task runs
//! (`runtime/blocking/schedule.rs`: `inhibit_auto_advance` at spawn,
//! `allow_auto_advance` and an unpark at release). The pump waits for
//! [`Busy`] to drain before it delivers, so time moves, and events land,
//! only when every task, database calls included, waits.

use std::cell::RefCell;
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};
use tokio::sync::Notify;

/// The database calls in flight from one simulation's runtime.
#[derive(Debug, Default)]
pub(crate) struct Busy {
    /// Calls in flight, and how many calls have begun.
    state: Mutex<(usize, u64)>,
    /// Signalled when the count drops to zero (the blocking waiter).
    drained: Condvar,
    /// Notified when the count drops to zero (the pump).
    idle: Notify,
}

impl Busy {
    /// How many calls have begun: a change across a settle means a task
    /// started database work.
    pub(crate) fn begun(&self) -> u64 {
        self.state.lock().1
    }

    /// Wait until no call is in flight.
    pub(crate) async fn drained(&self) {
        loop {
            let idle = self.idle.notified();
            if self.state.lock().0 == 0 {
                return;
            }
            idle.await;
        }
    }

    fn begin(self: &Arc<Self>) -> DbCall {
        let first = {
            let mut state = self.state.lock();
            state.0 += 1;
            state.1 += 1;
            state.0 == 1
        };
        if first {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let busy = Arc::clone(self);
                // Detached: it ends when the count drains; while it runs the
                // runtime's clock does not auto-advance.
                drop(runtime.spawn_blocking(move || {
                    let mut state = busy.state.lock();
                    while state.0 > 0 {
                        busy.drained.wait(&mut state);
                    }
                }));
            }
        }
        DbCall(Arc::clone(self))
    }
}

/// One database call in flight; drop it on the database thread once the
/// call has run and its result is sent.
#[derive(Debug)]
pub struct DbCall(Arc<Busy>);

impl Drop for DbCall {
    fn drop(&mut self) {
        let drained = {
            let mut state = self.0.state.lock();
            state.0 -= 1;
            state.0 == 0
        };
        if drained {
            self.0.drained.notify_all();
            self.0.idle.notify_one();
        }
    }
}

thread_local! {
    /// The simulation whose runtime runs on this thread.
    static CURRENT: RefCell<Option<Arc<Busy>>> = const { RefCell::new(None) };
}

/// Count this thread's database calls in `busy` (a simulation's runtime
/// thread: a current-thread runtime polls every task on it).
pub(crate) fn install(busy: &Arc<Busy>) {
    CURRENT.with(|current| *current.borrow_mut() = Some(Arc::clone(busy)));
}

/// A database call queued on this thread begins: `None` unless the thread
/// runs a simulation ([`crate::Netsim::new`]).
#[must_use]
pub fn db_call() -> Option<DbCall> {
    CURRENT.with(|current| current.borrow().as_ref().map(Busy::begin))
}
