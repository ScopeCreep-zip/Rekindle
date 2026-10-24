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

/// After a call on the database thread: when its outermost commit erased
/// key material, queue the WAL truncation behind the calls already
/// waiting, never inside the call that erased (`crate::erase`, I14).
fn after_erase(queue: &Connection, conn: &rusqlite::Connection) {
    if crate::erase::take_committed(conn)
        && queue
            .call_detached(|conn| crate::erase::truncate_wal(conn))
            .is_err()
    {
        tracing::debug!("database closing; its close checkpoints the erased frames");
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
        let after = self.conn.clone();
        self.conn
            .call(move |conn| {
                let result = function(conn);
                after_erase(&after, conn);
                result
            })
            .await
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
        let after = self.conn.clone();
        self.conn.call_detached(move |conn| {
            function(conn);
            after_erase(&after, conn);
        })
    }

    /// Fire-and-forget write: queued in order, a failure is logged.
    pub fn fire<F>(&self, context: &'static str, function: F)
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<(), rusqlite::Error> + Send + 'static,
    {
        let after = self.conn.clone();
        let queued = self.conn.call_detached(move |conn| {
            if let Err(e) = function(conn) {
                tracing::warn!(context, error = %e, "fire-and-forget DB operation failed");
            }
            after_erase(&after, conn);
        });
        if queued.is_err() {
            tracing::warn!(
                context,
                "fire-and-forget DB operation dropped: database closed"
            );
        }
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
