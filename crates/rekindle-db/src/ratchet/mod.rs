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

use rekindle_crypto::signal::{Outgoing, PreKeyBundle, PreKeyInit, RatchetMessage, SessionRecord};
use rekindle_crypto::{CryptoError, Identity};
use rekindle_utils::random::RandomSource;
use rusqlite::{params, Connection, OptionalExtension};

use crate::prekeys::{self, PrekeyKind};
use crate::repo::unprocessed::{self, UnprocessedKey};
use crate::repo::{identity_text, signal_identities};
use crate::Db;
use rekindle_crypto::signal::prekey_record::Publish;

mod peek;
mod tx;
pub use peek::{has_session, is_established, is_known_init};
use tx::ended;
pub(crate) use tx::in_transaction;

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
/// `ended_session`: the decrypt tripped the archive's breaker (plan E2.5h),
/// so no session is active and the caller shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inbound {
    pub key: UnprocessedKey,
    pub plaintext: Vec<u8>,
    pub ended_session: bool,
}

/// A decrypted ephemeral message; `ended_session` as for [`Inbound`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ephemeral {
    pub plaintext: Vec<u8>,
    pub ended_session: bool,
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
    /// A `SessionResetAccept` init from a peer we asked no reset of (B11;
    /// plan audit D-4): refused before any prekey is looked up.
    #[error("refused a session reset we did not request")]
    UnrequestedReset,
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
            local.now_ms,
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

/// Encrypt under the active session with `peer`: a ratchet message, or,
/// while they have not acknowledged the session, the message wrapped in
/// its PreKeyInit (plan E2.5h).
///
/// # Errors
/// See [`RatchetError`]; no session, or one unacknowledged past
/// `PREKEY_WRAP_MAX_AGE_MS`, is [`CryptoError::NoSession`].
pub async fn encrypt_and_record_outbound(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    plaintext: &[u8],
    ad_extra: &[u8],
) -> Result<Outgoing, RatchetError> {
    let (local, peer) = (local.clone(), peer.clone());
    let (plaintext, ad_extra) = (plaintext.to_vec(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let (next, outgoing) =
            load(conn, &owner, &peer)?.encrypt(&plaintext, &ad_extra, local.now_ms)?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        Ok(outgoing)
    })
    .await
}

/// [`encrypt_and_record_outbound`] for a user-content message of kind
/// `type_id`, logged in `message_send_log` in the same transaction under
/// its header id and sending ratchet key (plan E2.5, D24), so a
/// `RetryRequest` can have it resent, with its `client_id` so a
/// disappearing message's expiry deletes the row (plan E7.1).
///
/// # Errors
/// See [`RatchetError`].
pub async fn encrypt_record_and_log_outbound(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    plaintext: &[u8],
    ad_extra: &[u8],
    type_id: u16,
    client_id: Option<String>,
) -> Result<Outgoing, RatchetError> {
    let (local, peer) = (local.clone(), peer.clone());
    let (plaintext, ad_extra) = (plaintext.to_vec(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let (next, outgoing) =
            load(conn, &owner, &peer)?.encrypt(&plaintext, &ad_extra, local.now_ms)?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        let message = outgoing.message();
        let sent = crate::repo::send_log::LoggedSend {
            peer: peer.identity,
            msg_id: rekindle_crypto::replay::ratchet_message_id(
                &message.header.dh,
                message.header.n,
            ),
            type_id,
            ratchet_key: message.header.dh,
            plaintext,
            sent_at: local.now_ms,
            client_id,
        };
        crate::repo::send_log::insert(conn, &owner, &sent)?;
        Ok(outgoing)
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
        let record = load(conn, &owner, &peer)?;
        let (next, plaintext) =
            record.decrypt(&message, &ad_extra, local.now_ms, local.rng.as_ref())?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        let mut inbound = record_inbound(conn, &local, &peer, &message, type_id, plaintext)?;
        inbound.ended_session = ended(&record, &next);
        Ok(inbound)
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
) -> Result<Ephemeral, RatchetError> {
    let (local, peer, ad_extra) = (local.clone(), peer.clone(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let record = load(conn, &owner, &peer)?;
        let (next, plaintext) =
            record.decrypt(&message, &ad_extra, local.now_ms, local.rng.as_ref())?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        let ended_session = ended(&record, &next);
        Ok(Ephemeral {
            plaintext,
            ended_session,
        })
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
    /// Yes: the init is the `FriendAccept` of a friendship the user asked
    /// for (the friend gate admits it only from a peer we sent a request
    /// to, or befriended), so a session kept from an ended friendship does
    /// not refuse it (plan E2.5h: removal never deletes sessions).
    Befriended,
}

/// Accept `peer`'s PreKeyInit (plan E2.4 B8, E2.5h). An init whose base key
/// is a session's we hold is decrypted by that session in place, with no
/// prekey consumed. Otherwise the order is: build the session from the
/// prekeys it names and decrypt its first message without touching the
/// record; commit it (active or archived, the crossing tiebreak); the
/// ML-KEM replay gate; the identity; store the session; consume the
/// one-time keys and refill the slot; record the plaintext in
/// `unprocessed` as `type_id`. All of it is one transaction, so a failure
/// stores nothing. The commit truncates the WAL (`crate::erase`): the
/// erased one-time secrets and any session evicted from the archive must
/// not linger in `-wal` frames.
///
/// # Errors
/// See [`RatchetError`]:
/// - an init for a session we hold whose message was already decrypted is
///   [`CryptoError::DuplicateMessage`] (B4), dropped silently;
/// - an init under another identity than `peer`'s is
///   [`CryptoError::IdentityAddressMismatch`];
/// - a new session over an established one, unless `reinit` allows it, is
///   [`RatchetError::EstablishedSession`] (ruling Q1: nothing re-initiates
///   silently);
/// - a consumed, unknown or archived prekey is
///   [`RatchetError::UnknownPrekey`] (B5);
/// - a last-resort-only init seen before is
///   [`RatchetError::ReplayedLastResortInit`] (B9), dropped silently. It is
///   judged after the first message decrypts, so an unauthenticated init
///   cannot probe the replay set.
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
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let record = load(conn, &owner, &peer)?;
        if init.identity_key != peer.identity {
            return Err(CryptoError::IdentityAddressMismatch {
                address: hex::encode(peer.identity),
            }
            .into());
        }
        if record.has_base_key(&init.base_key) {
            let (next, plaintext) =
                record.receive_known_init(&init, &ad_extra, local.now_ms, local.rng.as_ref())?;
            store(conn, &owner, &peer, &next, local.now_ms)?;
            let mut inbound =
                record_inbound(conn, &local, &peer, &init.message, type_id, plaintext)?;
            inbound.ended_session = ended(&record, &next);
            return Ok(Accepted {
                inbound,
                refills: Vec::new(),
            });
        }
        if reinit == Reinitiation::Refused && record.is_established() {
            return Err(RatchetError::EstablishedSession);
        }
        // A reset's init answers our stored request, consumed in this
        // transaction (plan audit D-4).
        if reinit == Reinitiation::Requested
            && !crate::repo::session_reset_requests::take(
                conn,
                &identity_text(&owner),
                &identity_text(&peer.identity),
            )?
        {
            return Err(RatchetError::UnrequestedReset);
        }
        let secrets = prekeys::responder_secrets(conn, &owner, &init)?;
        let (next, plaintext) = record.insert_from_init(
            &init,
            &secrets.keys(&local.identity),
            &ad_extra,
            local.now_ms,
            local.rng.as_ref(),
        )?;
        if prekeys::is_last_resort_only(&init) && prekeys::lr_init_seen(conn, &owner, &init)? {
            return Err(RatchetError::ReplayedLastResortInit);
        }
        prekeys::record_lr_init(conn, &owner, &init, local.now_ms)?;
        signal_identities::ensure(conn, &owner, &peer.identity)?;
        store(conn, &owner, &peer, &next, local.now_ms)?;
        let consumed = prekeys::consume_one_time(conn, &owner, &init, local.now_ms)?;
        let owner_text = identity_text(&owner);
        let refills = consumed
            .into_iter()
            .map(|subkey| prekeys::fill_slot(conn, &owner_text, &local, subkey))
            .collect::<Result<_, _>>()?;
        let mut inbound = record_inbound(conn, &local, &peer, &init.message, type_id, plaintext)?;
        inbound.ended_session = ended(&record, &next);
        Ok(Accepted { inbound, refills })
    })
    .await
}

/// B6: the peer refused the prekeys our init with `base_key_hash` named.
/// Discard that session, but only while it is our unanswered init (nothing
/// decrypted under it), and `true`: no session is active then (an archived
/// one is never made active again, plan E2.5h), and the discarded keys are
/// erased. Anything else changes nothing.
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

/// End every session with `peer`: a pairwise session with a former group
/// member who is no friend of ours (plan E2.6). Never a removal, block or
/// re-add of a friend, which keep their sessions (plan E2.5h). The deleted
/// keys are erased through `crate::erase`, so the WAL is truncated after
/// the commit.
///
/// # Errors
/// The database refused.
pub async fn forget(db: &Db, owner: &[u8; 32], peer: &[u8; 32]) -> Result<(), RatchetError> {
    let (owner, peer) = (identity_text(owner), identity_text(peer));
    db.call(move |conn| {
        crate::erase::delete(
            conn,
            "DELETE FROM ratchet_sessions WHERE owner_key = ?1 AND peer_identity = ?2",
            params![owner, peer],
        )
        .map(|_| ())
    })
    .await
    .map_err(RatchetError::from)
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
    // Every write replaces the record before it: a step's spent chain and
    // skipped keys, an archived state the cap evicts, a session replaced
    // by a new one are all erased (I14, RV5 L2).
    crate::erase::erased();
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
    Ok(Inbound {
        key,
        plaintext,
        ended_session: false,
    })
}
