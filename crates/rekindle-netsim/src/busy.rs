//! Database work the paused clock can see (plan E2.B S8, findings F4/F7/F11).
//!
//! A Host's database runs on a `rekindle-asql` thread, outside tokio, so a
//! Host awaiting a call looks idle: tokio would auto-advance the paused
//! clock to the next timer while the thread works, and the runtime would
//! poll other tasks in an order set by how long the thread takes (F11: the
//! trace's tail moved with the database thread's timing). The harness
//! installs an asql call watch (`rekindle_db::watch_calls`) that takes a
//! [`db_call`] for each call and, once the call is queued, holds the
//! queuing thread in [`db_queued`] until the call has run. A call queued
//! from a simulation's runtime thread therefore runs while that runtime
//! polls nothing: no timer fires and no task moves until its result is
//! sent, so the calling task resumes at the same point of every run with
//! the same seed.

use std::cell::RefCell;
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};

/// The database calls in flight from one simulation's runtime.
#[derive(Debug, Default)]
pub(crate) struct Busy {
    /// Calls in flight, and how many calls have begun.
    state: Mutex<(usize, u64)>,
    /// Signalled when the count drops to zero.
    drained: Condvar,
}

impl Busy {
    /// How many calls have begun: a change across a settle means a task
    /// did database work.
    pub(crate) fn begun(&self) -> u64 {
        self.state.lock().1
    }

    fn begin(self: &Arc<Self>) -> DbCall {
        let mut state = self.state.lock();
        state.0 += 1;
        state.1 += 1;
        DbCall(Arc::clone(self))
    }

    /// Block until no call is in flight.
    fn wait(&self) {
        let mut state = self.state.lock();
        while state.0 > 0 {
            self.drained.wait(&mut state);
        }
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

/// A database call this thread began is queued: on a simulation's thread,
/// block until it (and any call before it) has run and sent its result.
/// The database thread needs nothing from the runtime, so this ends.
pub fn db_queued() {
    if let Some(busy) = CURRENT.with(|current| current.borrow().clone()) {
        busy.wait();
    }
}
