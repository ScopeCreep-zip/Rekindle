//! The ratchet's transactions: one per step on the `Db` thread, committed
//! only when the step succeeds. Erasure is not decided here: every record
//! write marks the connection erased (`store`, `crate::erase`), so an
//! archived session the cap evicts (plan E2.5h) leaves the WAL after the
//! outermost commit like every other erased key.

use rekindle_crypto::signal::SessionRecord;
use rusqlite::Connection;

use super::RatchetError;
use crate::Db;

/// Run `step` in one transaction on the `Db` thread; it commits only when
/// `step` succeeds.
pub(crate) async fn in_transaction<T, F>(db: &Db, step: F) -> Result<T, RatchetError>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T, RatchetError> + Send + 'static,
{
    db.call(move |conn| {
        let tx = conn.transaction()?;
        match step(&tx) {
            Ok(value) => {
                tx.commit()?;
                Ok(Ok(value))
            }
            Err(e) => Ok(Err(e)),
        }
    })
    .await?
}

/// Whether a step from `before` to `after` tripped the archive's breaker:
/// only it leaves a record with no active session after a decrypt.
pub(super) fn ended(before: &SessionRecord, after: &SessionRecord) -> bool {
    before.has_active() && !after.has_active()
}
