//! Our prekeys over `signal_prekeys` (plan E2.3): the bundles we hand to
//! peers, and the secrets a PreKeyInit names. They replace
//! `SignalSessionManager`'s minting (`rekindle-crypto`
//! `signal/session/bundles.rs`) with the same bundles in the same wire
//! form (`PreKeyBundle::to_wire`); E2.4 then replaces the bundle with the
//! prekey record.
//!
//! - [`current_bundle`]: the signed prekey and the PQ last-resort key,
//!   each minted once and never overwritten, and no one-time keys (PQXDH
//!   §3.3). For anything shared with more than one peer.
//! - [`handout_bundle`]: the current bundle plus a fresh one-time curve
//!   prekey and ML-KEM key under random non-zero ids unused for their kind
//!   (PQXDH §4.13), for a bundle sent to exactly one peer. Beyond
//!   [`MAX_UNCLAIMED_ONE_TIME`] unclaimed keys of a kind the oldest go.
//!
//! A one-time key is consumed only after the first message under it
//! decrypts (`crate::ratchet::accept_init`, X3DH §3.4): its secret is
//! erased and the row keeps the id taken.

use rekindle_crypto::signal::prekey_material::{
    mint_curve, mint_pq, signed_bundle, MintedPrekey, OneTimePublics, ResponderSecrets,
};
use rekindle_crypto::signal::{
    PqPrekeyId, PreKeyBundle, PreKeyInit, MAX_UNCLAIMED_ONE_TIME, PQ_LR_ID, SPK_ID,
};
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::ratchet::{LocalParty, RatchetError};
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

/// Our long-lived bundle (no one-time keys), minting its keys on first
/// use. One transaction.
///
/// # Errors
/// The database failed.
pub async fn current_bundle(db: &Db, local: &LocalParty) -> Result<PreKeyBundle, RatchetError> {
    let local = local.clone();
    crate::ratchet::in_transaction(db, move |conn| {
        let owner = local.owner();
        let (spk, lr) = long_lived(conn, &owner, &local)?;
        Ok(signed_bundle(&local.identity, &spk, &lr, None))
    })
    .await
}

/// A bundle for exactly one peer: the long-lived keys plus a fresh pair of
/// one-time keys, stored before it is returned. One transaction.
///
/// # Errors
/// The database failed.
pub async fn handout_bundle(db: &Db, local: &LocalParty) -> Result<PreKeyBundle, RatchetError> {
    let local = local.clone();
    crate::ratchet::in_transaction(db, move |conn| {
        let owner = local.owner();
        let (spk, lr) = long_lived(conn, &owner, &local)?;
        let opk = mint_curve(local.rng.as_ref());
        let opk_id = insert_one_time(conn, &owner, PrekeyKind::Opk, &opk, &local)?;
        let pq = mint_pq();
        let pq_id = insert_one_time(conn, &owner, PrekeyKind::PqOpk, &pq, &local)?;
        let one_time = OneTimePublics {
            opk_id,
            opk: &opk.public,
            pq_id,
            pq: &pq.public,
        };
        Ok(signed_bundle(&local.identity, &spk, &lr, Some(&one_time)))
    })
    .await
}

/// The secrets `init` names, unconsumed.
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
/// used (the secrets are erased; the rows keep their ids taken).
pub(crate) fn consume_one_time(
    conn: &Connection,
    owner: &[u8; 32],
    init: &PreKeyInit,
    now_ms: i64,
) -> rusqlite::Result<()> {
    let mut used = Vec::new();
    if let Some(id) = init.one_time_prekey_id {
        used.push((PrekeyKind::Opk, id));
    }
    if let PqPrekeyId::OneTime(id) = init.pq_prekey {
        used.push((PrekeyKind::PqOpk, id));
    }
    for (kind, id) in used {
        conn.execute(
            "UPDATE signal_prekeys SET secret = x'', consumed_at = ?4 \
             WHERE owner_key = ?1 AND kind = ?2 AND id = ?3 AND consumed_at IS NULL",
            params![identity_text(owner), kind.as_str(), id, now_ms],
        )?;
    }
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
    conn.query_row(
        "SELECT secret FROM signal_prekeys \
         WHERE owner_key = ?1 AND kind = ?2 AND id = ?3 AND consumed_at IS NULL",
        params![identity_text(owner), kind.as_str(), id],
        |row| row.get::<_, Vec<u8>>(0).map(Zeroizing::new),
    )
    .optional()?
    .ok_or(RatchetError::UnknownPrekey { kind, id })
}

/// The signed prekey's and the last-resort key's public halves, minted and
/// stored when absent.
fn long_lived(
    conn: &Connection,
    owner: &[u8; 32],
    local: &LocalParty,
) -> rusqlite::Result<(Vec<u8>, Vec<u8>)> {
    let spk = public_or_mint(conn, owner, PrekeyKind::Spk, SPK_ID, local, || {
        mint_curve(local.rng.as_ref())
    })?;
    let lr = public_or_mint(conn, owner, PrekeyKind::PqLr, PQ_LR_ID, local, mint_pq)?;
    Ok((spk, lr))
}

fn public_or_mint(
    conn: &Connection,
    owner: &[u8; 32],
    kind: PrekeyKind,
    id: u32,
    local: &LocalParty,
    mint: impl FnOnce() -> MintedPrekey,
) -> rusqlite::Result<Vec<u8>> {
    let stored = conn
        .query_row(
            "SELECT public FROM signal_prekeys WHERE owner_key = ?1 AND kind = ?2 AND id = ?3",
            params![identity_text(owner), kind.as_str(), id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()?;
    if let Some(public) = stored {
        return Ok(public);
    }
    let minted = mint();
    insert(conn, owner, kind, id, &minted, local.now_ms)?;
    Ok(minted.public)
}

/// Store a one-time key under a fresh id and evict the oldest unclaimed
/// keys of its kind beyond the cap.
fn insert_one_time(
    conn: &Connection,
    owner: &[u8; 32],
    kind: PrekeyKind,
    key: &MintedPrekey,
    local: &LocalParty,
) -> rusqlite::Result<u32> {
    let id = loop {
        let id = local.rng.next_u32();
        let taken: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM signal_prekeys \
             WHERE owner_key = ?1 AND kind = ?2 AND id = ?3)",
            params![identity_text(owner), kind.as_str(), id],
            |row| row.get(0),
        )?;
        if id != 0 && !taken {
            break id;
        }
    };
    insert(conn, owner, kind, id, key, local.now_ms)?;
    conn.execute(
        "DELETE FROM signal_prekeys WHERE rowid IN (\
           SELECT rowid FROM signal_prekeys \
           WHERE owner_key = ?1 AND kind = ?2 AND consumed_at IS NULL \
           ORDER BY created_at DESC, rowid DESC LIMIT -1 OFFSET ?3)",
        params![
            identity_text(owner),
            kind.as_str(),
            i64::try_from(MAX_UNCLAIMED_ONE_TIME).expect("the cap fits i64")
        ],
    )?;
    Ok(id)
}

fn insert(
    conn: &Connection,
    owner: &[u8; 32],
    kind: PrekeyKind,
    id: u32,
    key: &MintedPrekey,
    now_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO signal_prekeys (owner_key, id, kind, secret, public, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            identity_text(owner),
            id,
            kind.as_str(),
            key.secret.as_slice(),
            key.public,
            now_ms
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
