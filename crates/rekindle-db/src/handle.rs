//! `DbHandle`: the logged-in identity's database, or none (plan C6).
//!
//! Every command and service reaches the database through
//! [`DbHandle::current`], so "no identity is loaded" is an explicit error
//! rather than a database that is always there. Plan D1 sets and clears it
//! at login and logout; until then boot sets it once.

use parking_lot::RwLock;

use crate::Db;

/// No identity's database is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not logged in: no identity database is open")]
pub struct NotLoggedIn;

impl From<NotLoggedIn> for String {
    fn from(e: NotLoggedIn) -> Self {
        e.to_string()
    }
}

/// Clearing found other handles still in use. The handle is already empty,
/// so nobody new can reach the database; the connection closes when the
/// last of them drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the database is still held by {others} other handle(s); it closes when they drop")]
pub struct DbStillHeld {
    pub others: usize,
}

/// The current identity's database.
#[derive(Debug, Default)]
pub struct DbHandle(RwLock<Option<Db>>);

impl DbHandle {
    /// The open database.
    ///
    /// # Errors
    /// [`NotLoggedIn`] when none is set.
    pub fn current(&self) -> Result<Db, NotLoggedIn> {
        self.0.read().clone().ok_or(NotLoggedIn)
    }

    /// Install the database.
    ///
    /// # Panics
    /// One is already set: replacing a live database would leave its
    /// holders writing to a different identity's file.
    pub fn set(&self, db: Db) {
        let mut slot = self.0.write();
        assert!(slot.is_none(), "DbHandle::set with a database already set");
        *slot = Some(db);
    }

    /// Empty the handle and close the database. Nothing set is not an error.
    ///
    /// # Errors
    /// [`DbStillHeld`] when other handles are alive.
    pub async fn clear(&self) -> Result<(), DbStillHeld> {
        let Some(db) = self.0.write().take() else {
            return Ok(());
        };
        let others = db.handles() - 1;
        if others > 0 {
            return Err(DbStillHeld { others });
        }
        if let Err(e) = db.close().await {
            tracing::warn!(error = %e, "closing the identity database failed");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        Db::from(rekindle_asql::Connection::open_in_memory().await.unwrap())
    }

    #[tokio::test]
    async fn current_needs_a_set_database_and_clear_empties_it() {
        let handle = DbHandle::default();
        assert_eq!(handle.current().unwrap_err(), NotLoggedIn);
        handle.set(db().await);
        assert!(handle.current().is_ok());
        handle.clear().await.unwrap();
        assert_eq!(handle.current().unwrap_err(), NotLoggedIn);
        handle.clear().await.unwrap();
    }

    #[tokio::test]
    async fn clear_reports_handles_still_held_and_closes_when_they_drop() {
        let handle = DbHandle::default();
        handle.set(db().await);
        let held = handle.current().unwrap();
        assert_eq!(handle.clear().await, Err(DbStillHeld { others: 1 }));
        assert_eq!(handle.current().unwrap_err(), NotLoggedIn);
        assert_eq!(
            held.call(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)))
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    #[should_panic(expected = "already set")]
    async fn a_second_set_is_a_programming_error() {
        let handle = DbHandle::default();
        handle.set(db().await);
        handle.set(db().await);
    }
}
