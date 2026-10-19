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

use std::sync::Arc;

use rekindle_crypto::signal::{PreKeyBundle, PreKeyInit, RatchetMessage, SessionRecord};
use rekindle_crypto::{CryptoError, Identity};
use rekindle_utils::random::RandomSource;
use rusqlite::{params, Connection, OptionalExtension};

use crate::prekeys::{self, PrekeyKind};
use crate::repo::unprocessed::{self, UnprocessedKey};
use crate::repo::{identity_text, signal_identities};
use crate::Db;

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
    /// A PreKeyInit names a prekey we do not hold (or already consumed).
    #[error("no {kind} prekey {id}")]
    UnknownPrekey { kind: PrekeyKind, id: u32 },
}

impl From<rusqlite::Error> for RatchetError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.into())
    }
}

/// Start a session with `peer` from its `bundle`, encrypting the first
/// message; returns the PreKeyInit to send.
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
        let (next, plaintext) =
            load(conn, &owner, &peer)?.decrypt(&message, &ad_extra, local.rng.as_ref())?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        record_inbound(conn, &local, &peer, &message, type_id, plaintext)
    })
    .await
}

/// Accept `peer`'s PreKeyInit: build the session from the prekeys it names,
/// decrypt its first message, consume the one-time keys it used, and record
/// the plaintext in `unprocessed` as `type_id`.
///
/// # Errors
/// See [`RatchetError`]; an init whose base key is already a session's is
/// [`CryptoError::DuplicateMessage`], and one under another identity than
/// `peer`'s is [`CryptoError::IdentityAddressMismatch`].
pub async fn accept_init(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    init: PreKeyInit,
    ad_extra: &[u8],
    type_id: u16,
) -> Result<Inbound, RatchetError> {
    let (local, peer, ad_extra) = (local.clone(), peer.clone(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
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
        let secrets = prekeys::responder_secrets(conn, &owner, &init)?;
        let (next, plaintext) = record.insert_from_init(
            &init,
            &secrets.keys(&local.identity),
            &ad_extra,
            local.rng.as_ref(),
        )?;
        prekeys::consume_one_time(conn, &owner, &init, local.now_ms)?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        signal_identities::ensure(conn, &owner, &peer.identity)?;
        record_inbound(conn, &local, &peer, &init.message, type_id, plaintext)
    })
    .await
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
