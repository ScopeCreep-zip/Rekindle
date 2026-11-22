//! The connection's background thread, joined when the last handle drops
//! (finding DB1; see [`crate::Connection`]).

use std::{sync::Arc, thread};

/// The background thread, joined when the last handle drops.
pub(crate) struct Worker {
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    pub(crate) fn new(thread: thread::JoinHandle<()>) -> Arc<Self> {
        Arc::new(Self {
            thread: Some(thread),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        // A call running on the thread can hold the last handle (a clone
        // it captured); the thread then ends by itself once that call
        // returns, and cannot wait for its own end.
        if thread.thread().id() != thread::current().id() {
            // A panic on the thread (a call's, or rusqlite's own close
            // panic) was already reported there; the wait is only for
            // its end.
            let _ = thread.join();
        }
    }
}
