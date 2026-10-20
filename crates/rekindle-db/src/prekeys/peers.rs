//! The prekey bookkeeping about peers (plan E2.4): the slots of their
//! records we initiated from (B2's `prekey_slots_used`), and the
//! `PreKeyRejected` notices we sent them (B5's `prekey_rejections`).

use rekindle_crypto::signal::prekey_record::{UsedIds, REJECTIONS_PER_WINDOW, REJECTION_WINDOW_MS};
use rusqlite::{params, Connection};

use crate::ratchet::{in_transaction, PeerAddress, RatchetError};
use crate::repo::identity_text;
use crate::Db;

/// Record that we initiated to `peer` from its slot `opk_id` (B2), inside
/// the initiate's transaction: the pick never chooses it again while the
/// peer's record lists it.
pub(crate) fn insert_slot_used(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &PeerAddress,
    opk_id: u32,
    now_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO prekey_slots_used (owner_key, peer, device_id, opk_id, used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            identity_text(owner),
            identity_text(&peer.identity),
            peer.device,
            opk_id,
            now_ms
        ],
    )?;
    Ok(())
}

/// The slots of `peer`'s record we have used: B2's `UsedIds`.
///
/// # Errors
/// The database failed.
pub async fn used_slots(
    db: &Db,
    owner: &[u8; 32],
    peer: &PeerAddress,
) -> Result<UsedIds, RatchetError> {
    let (owner, identity, device) = (
        identity_text(owner),
        identity_text(&peer.identity),
        peer.device.clone(),
    );
    Ok(db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT opk_id FROM prekey_slots_used \
                 WHERE owner_key = ?1 AND peer = ?2 AND device_id = ?3",
            )?;
            let ids = stmt
                .query_map(params![owner, identity, device], |row| row.get(0))?
                .collect();
            ids
        })
        .await?)
}

/// Forget the used slots `peer`'s record no longer lists (`listed`, from
/// the force-refreshed read B2 makes); how many went.
///
/// # Errors
/// The database failed.
pub async fn sweep_used_slots(
    db: &Db,
    owner: &[u8; 32],
    peer: &PeerAddress,
    listed: Vec<u32>,
) -> Result<usize, RatchetError> {
    let (owner, identity, device) = (
        identity_text(owner),
        identity_text(&peer.identity),
        peer.device.clone(),
    );
    in_transaction(db, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT opk_id FROM prekey_slots_used \
             WHERE owner_key = ?1 AND peer = ?2 AND device_id = ?3",
        )?;
        let stored = stmt
            .query_map(params![owner, identity, device], |row| row.get::<_, u32>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut gone = 0;
        for id in stored.into_iter().filter(|id| !listed.contains(id)) {
            gone += conn.execute(
                "DELETE FROM prekey_slots_used \
                 WHERE owner_key = ?1 AND peer = ?2 AND device_id = ?3 AND opk_id = ?4",
                params![owner, identity, device, id],
            )?;
        }
        Ok(gone)
    })
    .await
}

/// Whether a `PreKeyRejected` may go (B5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionVerdict {
    /// Send it; it is recorded.
    Send,
    /// One already went for this base key.
    AlreadySent,
    /// The sender had [`REJECTIONS_PER_WINDOW`] in the last
    /// [`REJECTION_WINDOW_MS`].
    RateLimited,
}

/// B5's limits for a rejection of `sender`'s init with this base-key hash
/// (the hash the notice carries): one per base key and
/// [`REJECTIONS_PER_WINDOW`] per sender per window. A [`RejectionVerdict::Send`]
/// is recorded in the same transaction, so two concurrent checks cannot
/// both send.
///
/// # Errors
/// The database failed.
pub async fn admit_rejection(
    db: &Db,
    owner: &[u8; 32],
    sender: &[u8; 32],
    base_key_hash: [u8; 32],
    now_ms: i64,
) -> Result<RejectionVerdict, RatchetError> {
    let (owner, sender) = (identity_text(owner), identity_text(sender));
    in_transaction(db, move |conn| {
        let seen: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM prekey_rejections \
             WHERE owner_key = ?1 AND base_key_hash = ?2)",
            params![owner, base_key_hash.as_slice()],
            |row| row.get(0),
        )?;
        if seen {
            return Ok(RejectionVerdict::AlreadySent);
        }
        let recent: u32 = conn.query_row(
            "SELECT count(*) FROM prekey_rejections \
             WHERE owner_key = ?1 AND sender = ?2 AND sent_at > ?3",
            params![owner, sender, now_ms - REJECTION_WINDOW_MS],
            |row| row.get(0),
        )?;
        if recent >= REJECTIONS_PER_WINDOW {
            return Ok(RejectionVerdict::RateLimited);
        }
        conn.execute(
            "INSERT INTO prekey_rejections (owner_key, sender, base_key_hash, sent_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![owner, sender, base_key_hash.as_slice(), now_ms],
        )?;
        Ok(RejectionVerdict::Send)
    })
    .await
}
