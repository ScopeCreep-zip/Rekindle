//! The archive (plan E2.4): a replaced or consumed key's row is kept
//! [`RETIRED_ARCHIVE_MS`] so an init in flight still opens, then deleted
//! with its secret; the bookkeeping tables are swept at their horizons.
//! Maintenance runs the sweep each tick.

use rekindle_crypto::signal::prekey_record::{OPK_STALE_MS, RETIRED_ARCHIVE_MS};
use rusqlite::params;

use crate::ratchet::{in_transaction_erasing, RatchetError};
use crate::repo::identity_text;
use crate::Db;

/// What one sweep deleted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArchiveSweep {
    /// Retired or consumed prekey rows past the archive.
    pub prekeys: usize,
    /// Last-resort inits whose key is gone (B9's set).
    pub lr_inits: usize,
    /// Rejections past the archive horizon (B5).
    pub rejections: usize,
    /// Used-slot rows older than a slot lives (B2).
    pub used_slots: usize,
}

/// Sweep `owner`'s archive at `now_ms`, in one transaction:
/// - a prekey retired, or consumed, [`RETIRED_ARCHIVE_MS`] ago is deleted
///   (an init naming it is then `UnknownPrekey`, B5);
/// - a `prekey_lr_inits` row goes with its last-resort key, so B9's set
///   lives for the key's lifetime plus the archive;
/// - a `prekey_rejections` row goes after the archive horizon;
/// - a `prekey_slots_used` row goes after [`OPK_STALE_MS`], by which time
///   the peer has replaced that slot.
///
/// The commit truncates the WAL, so the deleted secrets do not linger in
/// its frames.
///
/// # Errors
/// The database failed.
pub async fn sweep_archive(
    db: &Db,
    owner: &[u8; 32],
    now_ms: i64,
) -> Result<ArchiveSweep, RatchetError> {
    let owner = identity_text(owner);
    let horizon = now_ms - RETIRED_ARCHIVE_MS;
    in_transaction_erasing(db, move |conn| {
        let prekeys = conn.execute(
            "DELETE FROM signal_prekeys WHERE owner_key = ?1 \
             AND (retired_at <= ?2 OR consumed_at <= ?2)",
            params![owner, horizon],
        )?;
        let lr_inits = conn.execute(
            "DELETE FROM prekey_lr_inits WHERE owner_key = ?1 AND NOT EXISTS (\
               SELECT 1 FROM signal_prekeys k WHERE k.owner_key = ?1 \
               AND k.kind = 'pq_lr' AND k.id = prekey_lr_inits.lr_id)",
            params![owner],
        )?;
        let rejections = conn.execute(
            "DELETE FROM prekey_rejections WHERE owner_key = ?1 AND sent_at <= ?2",
            params![owner, horizon],
        )?;
        let used_slots = conn.execute(
            "DELETE FROM prekey_slots_used WHERE owner_key = ?1 AND used_at <= ?2",
            params![owner, now_ms - OPK_STALE_MS],
        )?;
        Ok(ArchiveSweep {
            prekeys,
            lr_inits,
            rejections,
            used_slots,
        })
    })
    .await
}
