//! The responder's side of a PreKeyInit (plan E2.3, E2.4 B8/B9): the
//! secrets it names, their consumption once its first message decrypts,
//! and the last-resort inits seen (B9's replay set).

use rekindle_crypto::signal::prekey_material::ResponderSecrets;
use rekindle_crypto::signal::prekey_record::is_prekey_id;
use rekindle_crypto::signal::{PqPrekeyId, PreKeyInit};
use rekindle_types::dht_layout::prekeys::is_slot;
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use super::PrekeyKind;
use crate::ratchet::RatchetError;
use crate::repo::identity_text;

/// The secrets `init` names, unconsumed. A retired signed prekey or
/// last-resort key still opens until the archive sweep deletes it; after
/// that, and for a consumed or unknown one-time key, the init is
/// [`RatchetError::UnknownPrekey`] (B5).
pub(crate) fn responder_secrets(
    conn: &Connection,
    owner: &[u8; 32],
    init: &PreKeyInit,
) -> Result<ResponderSecrets, RatchetError> {
    let spk = secret(conn, owner, PrekeyKind::Spk, init.signed_prekey_id)?;
    let opk = init
        .one_time_prekey_id
        .map(|id| secret(conn, owner, PrekeyKind::Opk, id))
        .transpose()?;
    let (pq_kind, pq_id) = pq_slot(init.pq_prekey);
    let pq = secret(conn, owner, pq_kind, pq_id)?;
    Ok(ResponderSecrets::from_bytes(
        &spk,
        opk.as_deref().map(Vec::as_slice),
        &pq,
    )?)
}

/// The first message under `init` decrypted: consume the one-time keys it
/// used (the secrets are erased; the rows keep their ids taken until the
/// archive). Returns the slots to refill: those whose keys were the
/// slot's current ones (a retired key's slot already holds new keys).
pub(crate) fn consume_one_time(
    conn: &Connection,
    owner: &[u8; 32],
    init: &PreKeyInit,
    now_ms: i64,
) -> rusqlite::Result<Vec<u32>> {
    let mut used = Vec::new();
    if let Some(id) = init.one_time_prekey_id {
        used.push((PrekeyKind::Opk, id));
    }
    if let PqPrekeyId::OneTime(id) = init.pq_prekey {
        used.push((PrekeyKind::PqOpk, id));
    }
    let owner = identity_text(owner);
    let mut refill = Vec::new();
    for (kind, id) in used {
        let slot: Option<(u32, bool)> = conn
            .query_row(
                "UPDATE signal_prekeys SET secret = x'', consumed_at = ?4 \
                 WHERE owner_key = ?1 AND kind = ?2 AND id = ?3 AND consumed_at IS NULL \
                 RETURNING published_subkey, retired_at IS NULL",
                params![owner, kind.as_str(), id, now_ms],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((subkey, current)) = slot {
            if current && is_slot(subkey) && !refill.contains(&subkey) {
                refill.push(subkey);
            }
        }
    }
    Ok(refill)
}

/// Whether `init` opens on the last-resort key alone (no one-time curve
/// key): the kind of init B9's replay set guards, since nothing it names
/// is consumed.
pub(crate) fn is_last_resort_only(init: &PreKeyInit) -> bool {
    init.one_time_prekey_id.is_none() && matches!(init.pq_prekey, PqPrekeyId::LastResort(_))
}

/// Whether an init with `init`'s `(lr_id, spk_id, base_key)` already
/// opened a session (B9).
pub(crate) fn lr_init_seen(
    conn: &Connection,
    owner: &[u8; 32],
    init: &PreKeyInit,
) -> rusqlite::Result<bool> {
    let PqPrekeyId::LastResort(lr_id) = init.pq_prekey else {
        return Ok(false);
    };
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM prekey_lr_inits \
         WHERE owner_key = ?1 AND lr_id = ?2 AND spk_id = ?3 AND base_key = ?4)",
        params![
            identity_text(owner),
            lr_id,
            init.signed_prekey_id,
            init.base_key.as_slice()
        ],
        |row| row.get(0),
    )
}

/// Record that `init` opened a session on the last-resort key (B8's
/// record of LR use, B9's replay set).
pub(crate) fn record_lr_init(
    conn: &Connection,
    owner: &[u8; 32],
    init: &PreKeyInit,
    now_ms: i64,
) -> rusqlite::Result<()> {
    let PqPrekeyId::LastResort(lr_id) = init.pq_prekey else {
        return Ok(());
    };
    conn.execute(
        "INSERT OR IGNORE INTO prekey_lr_inits (owner_key, lr_id, spk_id, base_key, seen_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            identity_text(owner),
            lr_id,
            init.signed_prekey_id,
            init.base_key.as_slice(),
            now_ms
        ],
    )?;
    Ok(())
}

fn pq_slot(id: PqPrekeyId) -> (PrekeyKind, u32) {
    match id {
        PqPrekeyId::OneTime(id) => (PrekeyKind::PqOpk, id),
        PqPrekeyId::LastResort(id) => (PrekeyKind::PqLr, id),
    }
}

/// An unconsumed secret, or [`RatchetError::UnknownPrekey`].
fn secret(
    conn: &Connection,
    owner: &[u8; 32],
    kind: PrekeyKind,
    id: u32,
) -> Result<Zeroizing<Vec<u8>>, RatchetError> {
    if !is_prekey_id(id) {
        return Err(RatchetError::UnknownPrekey { kind, id });
    }
    conn.query_row(
        "SELECT secret FROM signal_prekeys \
         WHERE owner_key = ?1 AND kind = ?2 AND id = ?3 AND consumed_at IS NULL",
        params![identity_text(owner), kind.as_str(), id],
        |row| row.get::<_, Vec<u8>>(0).map(Zeroizing::new),
    )
    .optional()?
    .ok_or(RatchetError::UnknownPrekey { kind, id })
}
