//! Our prekeys over `signal_prekeys` (plan E2.3, E2.4): the rows behind
//! our prekey record, and the secrets a PreKeyInit names.
//!
//! The record (`rekindle_types::dht_layout::prekeys`) is a head at subkey
//! 0 (the signed prekey and the ML-KEM last-resort key) and one slot of
//! one-time keys at each of subkeys 1..=100. Every row names the subkey
//! publishing it (`published_subkey`). The decisions are
//! `rekindle_crypto::signal::prekey_record`'s; this module stores and
//! applies them, each call one transaction on the `Db` thread, and hands
//! back the [`Publish`] values the caller writes to the record **after**
//! the commit (a DHT write cannot join the transaction; the unlock
//! reconciliation, [`published_values`], closes a crash in between):
//!
//! - [`store_set`] stores a whole minted set (a new identity's; `rekey`
//!   does the same inside its transaction).
//! - [`local_prekeys`] is the input of `plan_maintenance`, and
//!   [`apply_maintenance`] applies its actions.
//! - [`published_values`] is what the record should hold, re-signed from
//!   the rows (Ed25519 is deterministic), for the reconciliation.
//! - [`status`] is the `PrekeyStatus` read.
//! - The responder's side of a PreKeyInit (`responder`), the bookkeeping
//!   about peers (`peers`) and the archive (`archive`).
//!
//! A one-time key is consumed only after the first message under it
//! decrypts (`crate::ratchet::accept_init`, B8): its secret is erased, and
//! the slot is refilled in the same transaction. A replaced key is
//! retired, not deleted, and stays usable until the archive sweep.

mod archive;
mod peers;
mod responder;
#[cfg(test)]
mod tests;

use rekindle_codec::capnp_codec::identity::{encode_opk_slot, encode_prekey_head};
use rekindle_crypto::signal::prekey_material::MintedPrekey;
use rekindle_crypto::signal::prekey_record::{
    mint_head, mint_slot, random_id, sign_head, sign_slot, LocalHead, LocalPrekeys, MintedSet,
    MintedSlot, PrekeyAction, SlotState, OPK_STALE_MS,
};
use rekindle_types::dht_layout::prekeys::{is_slot, HEAD};
use rusqlite::{params, Connection, OptionalExtension};

pub use archive::{sweep_archive, ArchiveSweep};
pub(crate) use peers::insert_slot_used;
pub use peers::{admit_rejection, sweep_used_slots, used_slots, RejectionVerdict};
pub(crate) use responder::{
    consume_one_time, is_last_resort_only, lr_init_seen, record_lr_init, responder_secrets,
};

use crate::ratchet::{in_transaction, LocalParty, RatchetError};
use crate::repo::identity_text;
use crate::Db;

/// A `signal_prekeys.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrekeyKind {
    /// A one-time X25519 prekey.
    Opk,
    /// The signed X25519 prekey.
    Spk,
    /// A one-time ML-KEM-768 prekey.
    PqOpk,
    /// The ML-KEM-768 last-resort prekey.
    PqLr,
}

impl PrekeyKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Opk => "opk",
            Self::Spk => "spk",
            Self::PqOpk => "pq_opk",
            Self::PqLr => "pq_lr",
        }
    }
}

impl std::fmt::Display for PrekeyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One value to write to our prekey record: the packed `PrekeyRecordHead`
/// at [`HEAD`] or an `OpkSlot` at a slot subkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    pub subkey: u32,
    pub value: Vec<u8>,
}

/// The values a minted set publishes: the head, then every slot.
#[must_use]
pub fn set_values(set: &MintedSet) -> Vec<Publish> {
    std::iter::once(Publish {
        subkey: HEAD,
        value: encode_prekey_head(&set.head.record),
    })
    .chain(set.slots.iter().map(slot_value))
    .collect()
}

/// Our prekeys at a glance (the `PrekeyStatus` read): the current head's
/// ids and ages, the fresh slots, and the consumed one-time keys the
/// archive still holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusRow {
    pub spk_id: u32,
    pub spk_age_ms: i64,
    pub pq_lr_id: u32,
    pub lr_age_ms: i64,
    pub fresh_slots: u32,
    pub consumed_slots: u32,
}

/// Store a whole minted set for `owner`, which has no prekeys yet (a new
/// identity). Every row is created at the head's `issuedAt`. One
/// transaction; returns the values to publish.
///
/// # Errors
/// The database failed (an id already taken for its kind, no such
/// identity).
pub async fn store_set(
    db: &Db,
    owner: &[u8; 32],
    set: MintedSet,
) -> Result<Vec<Publish>, RatchetError> {
    let owner = identity_text(owner);
    in_transaction(db, move |conn| {
        store_set_rows(conn, &owner, &set)?;
        Ok(set_values(&set))
    })
    .await
}

/// [`store_set`]'s rows, inside the caller's transaction.
pub(crate) fn store_set_rows(
    conn: &Connection,
    owner: &str,
    set: &MintedSet,
) -> rusqlite::Result<()> {
    let at = set.head.record.issued_at;
    let head = &set.head;
    insert(
        conn,
        owner,
        PrekeyKind::Spk,
        head.spk_id,
        &head.spk,
        at,
        HEAD,
    )?;
    insert(
        conn,
        owner,
        PrekeyKind::PqLr,
        head.pq_lr_id,
        &head.pq_lr,
        at,
        HEAD,
    )?;
    for slot in &set.slots {
        insert_slot(conn, owner, slot, at)?;
    }
    Ok(())
}

/// Our record as stored: `plan_maintenance`'s input.
///
/// # Errors
/// The database failed.
pub async fn local_prekeys(db: &Db, owner: &[u8; 32]) -> Result<LocalPrekeys, RatchetError> {
    let owner = identity_text(owner);
    Ok(db.call(move |conn| load_local(conn, &owner)).await?)
}

/// Apply maintenance's `actions` in one transaction: each mints under
/// fresh ids, retires what it replaces, and yields the value to publish.
///
/// # Errors
/// The database failed, or an action names a subkey that is not a slot
/// ([`RatchetError::InvalidSlot`]); nothing is stored.
pub async fn apply_maintenance(
    db: &Db,
    local: &LocalParty,
    actions: Vec<PrekeyAction>,
) -> Result<Vec<Publish>, RatchetError> {
    let local = local.clone();
    in_transaction(db, move |conn| {
        let owner = identity_text(&local.owner());
        actions
            .into_iter()
            .map(|action| match action {
                PrekeyAction::RotateHead => rotate_head(conn, &owner, &local),
                PrekeyAction::FillSlot { subkey } => fill_slot(conn, &owner, &local, subkey),
            })
            .collect()
    })
    .await
}

/// What our record should hold: the current head and every live slot,
/// re-signed from the stored public keys. The unlock reconciliation
/// publishes each value the record does not match.
///
/// # Errors
/// The database failed, or a stored key is malformed.
pub async fn published_values(db: &Db, local: &LocalParty) -> Result<Vec<Publish>, RatchetError> {
    let local = local.clone();
    db.call(move |conn| {
        let owner = identity_text(&local.owner());
        let mut values = Vec::new();
        if let (Some(spk), Some(lr)) = (
            current(conn, &owner, PrekeyKind::Spk)?,
            current(conn, &owner, PrekeyKind::PqLr)?,
        ) {
            let head = sign_head(
                &local.identity,
                spk.id,
                &spk.public,
                lr.id,
                &lr.public,
                spk.created_at,
            );
            values.push(head.map(|head| Publish {
                subkey: HEAD,
                value: encode_prekey_head(&head),
            }));
        }
        for (subkey, opk, pq) in live_pairs(conn, &owner)? {
            let slot = sign_slot(&local.identity, opk.id, &opk.public, pq.id, &pq.public);
            values.push(slot.map(|slot| Publish {
                subkey,
                value: encode_opk_slot(&slot),
            }));
        }
        Ok(values.into_iter().collect::<Result<Vec<_>, _>>())
    })
    .await?
    .map_err(RatchetError::from)
}

/// The `PrekeyStatus` read at `now_ms`; `None` before a head exists.
///
/// # Errors
/// The database failed.
pub async fn status(
    db: &Db,
    owner: &[u8; 32],
    now_ms: i64,
) -> Result<Option<StatusRow>, RatchetError> {
    let owner = identity_text(owner);
    Ok(db
        .call(move |conn| {
            let (Some(spk), Some(lr)) = (
                current(conn, &owner, PrekeyKind::Spk)?,
                current(conn, &owner, PrekeyKind::PqLr)?,
            ) else {
                return Ok(None);
            };
            let fresh = live_pairs(conn, &owner)?
                .iter()
                .filter(|(_, opk, pq)| now_ms - opk.created_at.min(pq.created_at) < OPK_STALE_MS)
                .count();
            let consumed: u32 = conn.query_row(
                "SELECT count(*) FROM signal_prekeys \
                 WHERE owner_key = ?1 AND kind = 'opk' AND consumed_at IS NOT NULL",
                params![owner],
                |row| row.get(0),
            )?;
            Ok(Some(StatusRow {
                spk_id: spk.id,
                spk_age_ms: now_ms - spk.created_at,
                pq_lr_id: lr.id,
                lr_age_ms: now_ms - lr.created_at,
                fresh_slots: u32::try_from(fresh).expect("at most SLOT_COUNT slots"),
                consumed_slots: consumed,
            }))
        })
        .await?)
}

/// Refill `subkey` inside the caller's transaction: retire what it holds
/// and mint new one-time keys under fresh ids.
pub(crate) fn fill_slot(
    conn: &Connection,
    owner: &str,
    local: &LocalParty,
    subkey: u32,
) -> Result<Publish, RatchetError> {
    if !is_slot(subkey) {
        return Err(RatchetError::InvalidSlot { subkey });
    }
    conn.execute(
        "UPDATE signal_prekeys SET retired_at = ?3 \
         WHERE owner_key = ?1 AND published_subkey = ?2 AND kind IN ('opk', 'pq_opk') \
         AND retired_at IS NULL",
        params![owner, subkey, local.now_ms],
    )?;
    let opk_id = fresh_id(conn, owner, PrekeyKind::Opk, local)?;
    let pq_opk_id = fresh_id(conn, owner, PrekeyKind::PqOpk, local)?;
    let slot = mint_slot(
        &local.identity,
        local.rng.as_ref(),
        subkey,
        opk_id,
        pq_opk_id,
    );
    insert_slot(conn, owner, &slot, local.now_ms)?;
    Ok(slot_value(&slot))
}

fn rotate_head(
    conn: &Connection,
    owner: &str,
    local: &LocalParty,
) -> Result<Publish, RatchetError> {
    conn.execute(
        "UPDATE signal_prekeys SET retired_at = ?2 \
         WHERE owner_key = ?1 AND kind IN ('spk', 'pq_lr') AND retired_at IS NULL",
        params![owner, local.now_ms],
    )?;
    let spk_id = fresh_id(conn, owner, PrekeyKind::Spk, local)?;
    let pq_lr_id = fresh_id(conn, owner, PrekeyKind::PqLr, local)?;
    let head = mint_head(
        &local.identity,
        local.rng.as_ref(),
        spk_id,
        pq_lr_id,
        local.now_ms,
    );
    let at = local.now_ms;
    insert(conn, owner, PrekeyKind::Spk, spk_id, &head.spk, at, HEAD)?;
    insert(
        conn,
        owner,
        PrekeyKind::PqLr,
        pq_lr_id,
        &head.pq_lr,
        at,
        HEAD,
    )?;
    Ok(Publish {
        subkey: HEAD,
        value: encode_prekey_head(&head.record),
    })
}

fn slot_value(slot: &MintedSlot) -> Publish {
    Publish {
        subkey: slot.subkey,
        value: encode_opk_slot(&slot.record),
    }
}

/// A stored public key.
struct Row {
    id: u32,
    public: Vec<u8>,
    created_at: i64,
}

/// The current (unretired) key of a long-lived kind.
fn current(conn: &Connection, owner: &str, kind: PrekeyKind) -> rusqlite::Result<Option<Row>> {
    conn.query_row(
        "SELECT id, public, created_at FROM signal_prekeys \
         WHERE owner_key = ?1 AND kind = ?2 AND retired_at IS NULL \
         ORDER BY created_at DESC, rowid DESC LIMIT 1",
        params![owner, kind.as_str()],
        |row| {
            Ok(Row {
                id: row.get(0)?,
                public: row.get(1)?,
                created_at: row.get(2)?,
            })
        },
    )
    .optional()
}

/// Every slot whose curve and ML-KEM one-time keys are both live, as
/// `(subkey, opk, pq_opk)` in subkey order.
fn live_pairs(conn: &Connection, owner: &str) -> rusqlite::Result<Vec<(u32, Row, Row)>> {
    let mut stmt = conn.prepare(
        "SELECT o.published_subkey, o.id, o.public, o.created_at, p.id, p.public, p.created_at \
         FROM signal_prekeys o JOIN signal_prekeys p \
           ON p.owner_key = o.owner_key AND p.published_subkey = o.published_subkey \
         WHERE o.owner_key = ?1 AND o.kind = 'opk' AND p.kind = 'pq_opk' \
           AND o.consumed_at IS NULL AND o.retired_at IS NULL \
           AND p.consumed_at IS NULL AND p.retired_at IS NULL \
         ORDER BY o.published_subkey",
    )?;
    let rows = stmt
        .query_map(params![owner], |row| {
            Ok((
                row.get(0)?,
                Row {
                    id: row.get(1)?,
                    public: row.get(2)?,
                    created_at: row.get(3)?,
                },
                Row {
                    id: row.get(4)?,
                    public: row.get(5)?,
                    created_at: row.get(6)?,
                },
            ))
        })?
        .collect();
    rows
}

fn load_local(conn: &Connection, owner: &str) -> rusqlite::Result<LocalPrekeys> {
    let head = match (
        current(conn, owner, PrekeyKind::Spk)?,
        current(conn, owner, PrekeyKind::PqLr)?,
    ) {
        (Some(spk), Some(lr)) => Some(LocalHead {
            spk_id: spk.id,
            pq_lr_id: lr.id,
            issued_at_ms: spk.created_at.min(lr.created_at),
        }),
        _ => None,
    };
    let mut slots: std::collections::BTreeMap<_, _> = live_pairs(conn, owner)?
        .into_iter()
        .map(|(subkey, opk, pq)| {
            let created_at_ms = opk.created_at.min(pq.created_at);
            (subkey, SlotState::Live { created_at_ms })
        })
        .collect();
    // A slot with unretired rows but no live pair: its keys were consumed
    // (or half of them) and nothing replaced them.
    let mut stmt = conn.prepare(
        "SELECT DISTINCT published_subkey FROM signal_prekeys \
         WHERE owner_key = ?1 AND kind IN ('opk', 'pq_opk') AND retired_at IS NULL",
    )?;
    for subkey in stmt.query_map(params![owner], |row| row.get::<_, u32>(0))? {
        slots.entry(subkey?).or_insert(SlotState::Consumed);
    }
    Ok(LocalPrekeys { head, slots })
}

/// A random id no row of `kind` holds (PQXDH §4.13).
fn fresh_id(
    conn: &Connection,
    owner: &str,
    kind: PrekeyKind,
    local: &LocalParty,
) -> rusqlite::Result<u32> {
    loop {
        let id = random_id(local.rng.as_ref());
        let taken: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM signal_prekeys \
             WHERE owner_key = ?1 AND kind = ?2 AND id = ?3)",
            params![owner, kind.as_str(), id],
            |row| row.get(0),
        )?;
        if !taken {
            return Ok(id);
        }
    }
}

fn insert_slot(conn: &Connection, owner: &str, slot: &MintedSlot, at: i64) -> rusqlite::Result<()> {
    let subkey = slot.subkey;
    insert(
        conn,
        owner,
        PrekeyKind::Opk,
        slot.opk_id,
        &slot.opk,
        at,
        subkey,
    )?;
    insert(
        conn,
        owner,
        PrekeyKind::PqOpk,
        slot.pq_opk_id,
        &slot.pq_opk,
        at,
        subkey,
    )
}

fn insert(
    conn: &Connection,
    owner: &str,
    kind: PrekeyKind,
    id: u32,
    key: &MintedPrekey,
    created_at: i64,
    subkey: u32,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO signal_prekeys \
         (owner_key, id, kind, secret, public, created_at, published_subkey) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            owner,
            id,
            kind.as_str(),
            key.secret.as_slice(),
            key.public,
            created_at,
            subkey
        ],
    )?;
    Ok(())
}
