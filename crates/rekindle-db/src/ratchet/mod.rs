//! The 1:1 ratchet over `ratchet_sessions` (plan E2.3): each function
//! loads the peer's `SessionRecord`, runs one pure step, and stores the new
//! record in one transaction on the `Db` thread, **before** the caller
//! sends anything.
//!
//! - A crash after the commit and before the send loses nothing: the
//!   retry sends the same bytes, and the next message uses the next key.
//! - The `Db` thread runs calls one at a time, in order
//!   (`rekindle_asql::Connection`), so concurrent messages for one peer
//!   are serialized by construction: one load per peer, no lost update
//!   (the C8 finding, re-homed).
//! - Inbound plaintext is written to `unprocessed` in the same
//!   transaction as the ratchet advance (D24); the apply deletes it
//!   (`repo::unprocessed`).
//! - A failure changes nothing: the transaction is dropped, and the record,
//!   the prekeys and `unprocessed` are as they were. A duplicate is
//!   [`CryptoError::DuplicateMessage`], which the receiver drops silently.
//! - A PreKeyInit's prekeys are consumed in the transaction that stores its
//!   session, only after its first message decrypts, and the slot is
//!   refilled there too (plan E2.4 B8, [`accept_init`]).

use std::sync::Arc;

use rekindle_crypto::signal::{PreKeyBundle, PreKeyInit, RatchetMessage, SessionRecord};
use rekindle_crypto::{CryptoError, Identity};
use rekindle_utils::random::RandomSource;
use rusqlite::{params, Connection, OptionalExtension};

use crate::prekeys::{self, PrekeyKind, Publish};
use crate::repo::unprocessed::{self, UnprocessedKey};
use crate::repo::{identity_text, signal_identities};
use crate::Db;

#[cfg(test)]
mod accept_tests;
#[cfg(test)]
mod tests;

/// The peer device a session is with.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerAddress {
    /// The peer's Ed25519 identity, which is its address.
    pub identity: [u8; 32],
    /// The device id (hex) from the peer's device list (E2.4), empty until
    /// a list names one.
    pub device: String,
}

impl PeerAddress {
    /// The peer's only device, before its device list names one.
    #[must_use]
    pub fn sole_device(identity: [u8; 32]) -> Self {
        Self {
            identity,
            device: String::new(),
        }
    }
}

/// Our side of a step: our identity, the generator the step draws from
/// (C9.2) and the clock reading it records, in milliseconds.
#[derive(Clone)]
pub struct LocalParty {
    pub identity: Arc<Identity>,
    pub rng: Arc<dyn RandomSource>,
    pub now_ms: i64,
}

impl LocalParty {
    /// Our identity key, which keys our rows.
    #[must_use]
    pub fn owner(&self) -> [u8; 32] {
        self.identity.public_key_bytes()
    }
}

/// A decrypted inbound message, recorded in `unprocessed` under `key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inbound {
    pub key: UnprocessedKey,
    pub plaintext: Vec<u8>,
}

/// An accepted PreKeyInit: its first message, and the refilled slots of
/// our prekey record. The caller writes each [`Publish`] to the record
/// after the commit (a durable pool write); the unlock reconciliation
/// re-publishes any a crash lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub inbound: Inbound,
    pub refills: Vec<Publish>,
}

/// A ratchet step failed; nothing was stored.
#[derive(Debug, thiserror::Error)]
pub enum RatchetError {
    /// The database refused, or is closed.
    #[error("database: {0}")]
    Db(#[from] rekindle_asql::Error),
    /// The session step failed: no session, a duplicate, a bad message, or
    /// an identity that is not the address.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    /// A PreKeyInit names a prekey we do not hold: never ours, already
    /// consumed, or past the archive. The init gets a `PreKeyRejected`
    /// when the friend gate admits its sender (plan E2.4 B5).
    #[error("no {kind} prekey {id}")]
    UnknownPrekey { kind: PrekeyKind, id: u32 },
    /// A PreKeyInit on the last-resort key alone whose `(lr_id, spk_id,
    /// base_key)` already opened a session: a replay, dropped silently
    /// with no rejection (plan E2.4 B9).
    #[error("replayed last-resort PreKeyInit")]
    ReplayedLastResortInit,
    /// A PreKeyInit from a peer we hold an established session with
    /// (something decrypted under it): a re-initiation we did not ask for,
    /// refused before any prekey is looked up (ruling Q1, plan audit G2).
    #[error("refused a re-initiation over an established session")]
    EstablishedSession,
    /// A maintenance action named a subkey that is not a one-time slot.
    #[error("subkey {subkey} is not a prekey slot")]
    InvalidSlot { subkey: u32 },
}

impl From<rusqlite::Error> for RatchetError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.into())
    }
}

/// Start a session with `peer` from its `bundle` (assembled from its
/// prekey record, B2), encrypting the first message; returns the
/// PreKeyInit to send. The slot the bundle came from is recorded as used
/// against `peer` in the same transaction, so no later pick reuses it.
///
/// # Errors
/// See [`RatchetError`]; a bundle under another key than `peer`'s is
/// [`CryptoError::IdentityAddressMismatch`].
pub async fn initiate_and_record_outbound(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    bundle: PreKeyBundle,
    first_plaintext: &[u8],
    ad_extra: &[u8],
) -> Result<PreKeyInit, RatchetError> {
    let (local, peer) = (local.clone(), peer.clone());
    let (plaintext, ad_extra) = (first_plaintext.to_vec(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let record = load(conn, &owner, &peer)?;
        let (next, init) = record.initiate(
            &local.identity,
            &bundle,
            &plaintext,
            &ad_extra,
            local.rng.as_ref(),
        )?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        signal_identities::ensure(conn, &owner, &peer.identity)?;
        if let Some(opk_id) = init.one_time_prekey_id {
            prekeys::insert_slot_used(conn, &owner, &peer, opk_id, local.now_ms)?;
        }
        Ok(init)
    })
    .await
}

/// Encrypt under the active session with `peer`.
///
/// # Errors
/// See [`RatchetError`]; no session is [`CryptoError::NoSession`].
pub async fn encrypt_and_record_outbound(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    plaintext: &[u8],
    ad_extra: &[u8],
) -> Result<RatchetMessage, RatchetError> {
    let (local, peer) = (local.clone(), peer.clone());
    let (plaintext, ad_extra) = (plaintext.to_vec(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let (next, message) = load(conn, &owner, &peer)?.encrypt(&plaintext, &ad_extra)?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        Ok(message)
    })
    .await
}

/// Decrypt a ratchet message from `peer` and record its plaintext in
/// `unprocessed` as `type_id`.
///
/// # Errors
/// See [`RatchetError`]; a message already decrypted is
/// [`CryptoError::DuplicateMessage`].
pub async fn decrypt_and_record_inbound(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    message: RatchetMessage,
    ad_extra: &[u8],
    type_id: u16,
) -> Result<Inbound, RatchetError> {
    let (local, peer, ad_extra) = (local.clone(), peer.clone(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let (next, plaintext) = load(conn, &owner, &peer)?.decrypt(
            &message,
            &ad_extra,
            local.now_ms,
            local.rng.as_ref(),
        )?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        record_inbound(conn, &local, &peer, &message, type_id, plaintext)
    })
    .await
}

/// Decrypt an ephemeral ratchet message from `peer` (typing, a call kind,
/// DM video; plan audit G10): the ratchet advances in one transaction as
/// for any message, but nothing goes to `unprocessed`, since a late one is
/// worth nothing and is never replayed.
///
/// # Errors
/// See [`RatchetError`]; a message already decrypted is
/// [`CryptoError::DuplicateMessage`].
pub async fn decrypt_ephemeral(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    message: RatchetMessage,
    ad_extra: &[u8],
) -> Result<Vec<u8>, RatchetError> {
    let (local, peer, ad_extra) = (local.clone(), peer.clone(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let (next, plaintext) = load(conn, &owner, &peer)?.decrypt(
            &message,
            &ad_extra,
            local.now_ms,
            local.rng.as_ref(),
        )?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        Ok(plaintext)
    })
    .await
}

/// Whether a PreKeyInit may replace an established session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reinitiation {
    /// No: a first contact, or a content kind opening a session after a
    /// rotation. Over an established session it is
    /// [`RatchetError::EstablishedSession`].
    Refused,
    /// Yes: the init answers a session reset we asked for (B11).
    Requested,
}

/// Accept `peer`'s PreKeyInit (plan E2.4 B8): build the session from the
/// prekeys it names and decrypt its first message; only then, in the same
/// transaction, consume the one-time keys it used, record a last-resort
/// use, store the session, refill the consumed slot and record the
/// plaintext in `unprocessed` as `type_id`. A failure stores nothing. The
/// commit truncates the WAL ([`truncate_wal`]): the erased one-time
/// secrets and the replaced session must not linger in `-wal` frames.
///
/// # Errors
/// See [`RatchetError`]:
/// - an init whose base key is already a session's is
///   [`CryptoError::DuplicateMessage`] (B4), dropped silently;
/// - an init under another identity than `peer`'s is
///   [`CryptoError::IdentityAddressMismatch`];
/// - an init over an established session, unless `reinit` is
///   [`Reinitiation::Requested`], is [`RatchetError::EstablishedSession`]
///   (ruling Q1: nothing re-initiates silently);
/// - a last-resort-only init seen before is
///   [`RatchetError::ReplayedLastResortInit`] (B9), dropped silently;
/// - a consumed, unknown or archived prekey is
///   [`RatchetError::UnknownPrekey`] (B5).
///
/// Every refusal comes before a prekey is looked up.
pub async fn accept_init(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    init: PreKeyInit,
    reinit: Reinitiation,
    ad_extra: &[u8],
    type_id: u16,
) -> Result<Accepted, RatchetError> {
    let (local, peer, ad_extra) = (local.clone(), peer.clone(), ad_extra.to_vec());
    in_transaction_erasing(db, move |conn| {
        let owner = local.owner();
        let record = load(conn, &owner, &peer)?;
        if record.has_base_key(&init.base_key) {
            return Err(CryptoError::DuplicateMessage.into());
        }
        if init.identity_key != peer.identity {
            return Err(CryptoError::IdentityAddressMismatch {
                address: hex::encode(peer.identity),
            }
            .into());
        }
        if reinit == Reinitiation::Refused && record.is_established() {
            return Err(RatchetError::EstablishedSession);
        }
        if prekeys::is_last_resort_only(&init) && prekeys::lr_init_seen(conn, &owner, &init)? {
            return Err(RatchetError::ReplayedLastResortInit);
        }
        let secrets = prekeys::responder_secrets(conn, &owner, &init)?;
        let (next, plaintext) = record.insert_from_init(
            &init,
            &secrets.keys(&local.identity),
            &ad_extra,
            local.now_ms,
            local.rng.as_ref(),
        )?;
        let consumed = prekeys::consume_one_time(conn, &owner, &init, local.now_ms)?;
        prekeys::record_lr_init(conn, &owner, &init, local.now_ms)?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        signal_identities::ensure(conn, &owner, &peer.identity)?;
        let owner_text = identity_text(&owner);
        let refills = consumed
            .into_iter()
            .map(|subkey| prekeys::fill_slot(conn, &owner_text, &local, subkey))
            .collect::<Result<_, _>>()?;
        let inbound = record_inbound(conn, &local, &peer, &init.message, type_id, plaintext)?;
        Ok(Accepted { inbound, refills })
    })
    .await
}

/// B6: the peer refused the prekeys our init with `base_key_hash` named.
/// Discard that session, but only while it is our unanswered init (nothing
/// decrypted under it): then the archived session it replaced is active
/// again, and `true`. Anything else changes nothing.
///
/// # Errors
/// The database refused.
pub async fn discard_pending_init(
    db: &Db,
    owner: &[u8; 32],
    peer: &PeerAddress,
    base_key_hash: [u8; 32],
    now_ms: i64,
) -> Result<bool, RatchetError> {
    let (owner, peer) = (*owner, peer.clone());
    in_transaction(db, move |conn| {
        let record = load(conn, &owner, &peer)?;
        let Some(base_key) = record.pending_init().copied() else {
            return Ok(false);
        };
        if rekindle_crypto::signal::prekey_record::base_key_hash(&base_key) != base_key_hash {
            return Ok(false);
        }
        let Some(next) = record.without_pending(&base_key) else {
            return Ok(false);
        };
        store(conn, &owner, &peer, &next, now_ms)?;
        Ok(true)
    })
    .await
}

/// End every session with `peer` (an unfriend, a block, a removal): the
/// next message to them initiates from their prekey record. Unprocessed messages
/// from them stay for their apply, which the friend gate then judges.
///
/// # Errors
/// The database refused.
pub async fn forget(db: &Db, owner: &[u8; 32], peer: &[u8; 32]) -> Result<(), RatchetError> {
    let (owner, peer) = (identity_text(owner), identity_text(peer));
    db.call(move |conn| {
        conn.execute(
            "DELETE FROM ratchet_sessions WHERE owner_key = ?1 AND peer_identity = ?2",
            params![owner, peer],
        )
        .map(|_| ())
    })
    .await
    .map_err(RatchetError::from)
}

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

/// [`in_transaction`], then a `wal_checkpoint(TRUNCATE)` once it commits:
/// for a step that erases secrets (a prekey consumption, the archive
/// sweep), so their old pages leave the `-wal` file as well as the
/// database (`secure_delete` zeroes the freed pages there; DR §8.1).
pub(crate) async fn in_transaction_erasing<T, F>(db: &Db, step: F) -> Result<T, RatchetError>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T, RatchetError> + Send + 'static,
{
    db.call(move |conn| {
        let tx = conn.transaction()?;
        match step(&tx) {
            Ok(value) => {
                tx.commit()?;
                truncate_wal(conn);
                Ok(Ok(value))
            }
            Err(e) => Ok(Err(e)),
        }
    })
    .await?
}

/// Checkpoint the WAL into the database and truncate it to zero bytes. The
/// step it follows has committed, so a failure (a reader holding the WAL)
/// is logged, not returned: the next truncating checkpoint, or the
/// connection's close, removes the frames.
pub(crate) fn truncate_wal(conn: &Connection) {
    match conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(0) => {}
        Ok(_) => tracing::warn!("WAL checkpoint blocked by a reader; erased frames remain"),
        Err(e) => tracing::warn!(error = %e, "WAL checkpoint failed; erased frames remain"),
    }
}

/// The stored record for `peer`, or an empty one.
fn load(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &PeerAddress,
) -> Result<SessionRecord, RatchetError> {
    let bytes = conn
        .query_row(
            "SELECT record FROM ratchet_sessions \
             WHERE owner_key = ?1 AND peer_identity = ?2 AND peer_device = ?3",
            params![
                identity_text(owner),
                identity_text(&peer.identity),
                peer.device
            ],
            |row| row.get::<_, Vec<u8>>(0).map(zeroize::Zeroizing::new),
        )
        .optional()?;
    let Some(bytes) = bytes else {
        return Ok(SessionRecord::new(peer.identity));
    };
    let record = SessionRecord::from_bytes(&bytes)?;
    if *record.peer_identity() != peer.identity {
        return Err(CryptoError::IdentityAddressMismatch {
            address: hex::encode(peer.identity),
        }
        .into());
    }
    Ok(record)
}

fn store(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &PeerAddress,
    record: &SessionRecord,
    now_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO ratchet_sessions (owner_key, peer_identity, peer_device, record, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (owner_key, peer_identity, peer_device) \
         DO UPDATE SET record = excluded.record, updated_at = excluded.updated_at",
        params![
            identity_text(owner),
            identity_text(&peer.identity),
            peer.device,
            record.to_bytes().as_slice(),
            now_ms
        ],
    )?;
    Ok(())
}

fn record_inbound(
    conn: &Connection,
    local: &LocalParty,
    peer: &PeerAddress,
    message: &RatchetMessage,
    type_id: u16,
    plaintext: Vec<u8>,
) -> Result<Inbound, RatchetError> {
    let key = UnprocessedKey {
        peer_identity: peer.identity,
        header_dh: message.header.dh,
        header_n: message.header.n,
    };
    unprocessed::insert(
        conn,
        &local.owner(),
        &key,
        type_id,
        &plaintext,
        local.now_ms,
    )?;
    Ok(Inbound { key, plaintext })
}
