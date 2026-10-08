//! `Db`: the handle every query goes through.

use std::sync::Arc;

use rekindle_asql::Connection;

/// A database handle: the connection's background thread, plus a liveness
/// token whose count says how many clones are still held (plan C6 checks
/// it before closing at logout). Cheap to clone.
#[derive(Clone)]
pub struct Db {
    conn: Connection,
    live: Arc<()>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("handles", &self.handles())
            .finish_non_exhaustive()
    }
}

impl From<Connection> for Db {
    fn from(conn: Connection) -> Self {
        Self {
            conn,
            live: Arc::new(()),
        }
    }
}

impl Db {
    /// Run `function` on the database thread and return its result.
    ///
    /// # Errors
    /// The connection is closed, or `function` failed.
    pub async fn call<F, R>(&self, function: F) -> rekindle_asql::Result<R>
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<R, rusqlite::Error> + 'static + Send,
        R: Send + 'static,
    {
        self.conn.call(function).await
    }

    /// Queue `function` on the database thread without waiting for it; it
    /// runs in order with every other call.
    ///
    /// # Errors
    /// The connection is closed.
    pub fn call_detached<F>(&self, function: F) -> rekindle_asql::Result<()>
    where
        F: FnOnce(&mut rusqlite::Connection) + Send + 'static,
    {
        self.conn.call_detached(function)
    }

    /// Fire-and-forget write: queued in order, a failure is logged.
    pub fn fire<F>(&self, context: &'static str, function: F)
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<(), rusqlite::Error> + Send + 'static,
    {
        let queued = self.conn.call_detached(move |conn| {
            if let Err(e) = function(conn) {
                tracing::warn!(context, error = %e, "fire-and-forget DB operation failed");
            }
        });
        if queued.is_err() {
            tracing::warn!(
                context,
                "fire-and-forget DB operation dropped: database closed"
            );
        }
    }

    /// Run a call whose failure is not an error to the caller: `T::default()`
    /// on any failure.
    pub async fn call_or_default<T, F>(&self, function: F) -> T
    where
        T: Send + Default + 'static,
        F: FnOnce(&mut rusqlite::Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    {
        self.conn.call(function).await.unwrap_or_default()
    }

    /// Close the connection: queued calls finish, later calls on any other
    /// handle fail with `ConnectionClosed`.
    ///
    /// # Errors
    /// SQLite refused to close.
    pub async fn close(self) -> rekindle_asql::Result<()> {
        self.conn.close().await
    }

    /// How many handles to this database are alive, this one included.
    #[must_use]
    pub fn handles(&self) -> usize {
        Arc::strong_count(&self.live)
    }
}
