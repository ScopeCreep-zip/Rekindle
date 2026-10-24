//! Opening an inbound `DmFrame` before anything acts on it (plan E2.3):
//! the block list, the friend gate for its kind, then its carrier's checks
//! — a sealed frame's signature, freshness and replay guard; a ratchet
//! message's decrypt; a PreKeyInit's session — and its body.
//!
//! A ratchet or PreKeyInit plaintext is recorded in `unprocessed` in the
//! same transaction as the ratchet advance (D24); the opened message names
//! its row, and the apply deletes it ([`processed`]). A sealed frame
//! advances no ratchet, so it is applied directly, behind the replay guard.
//! A failure on an established session never re-initiates on its own (r10
//! B10): the frame is dropped and logged.

use rekindle_codec::capnp_codec::direct::{
    DirectBody, DmContent, DmFrame, DmTypeId, FrameBody, PreKeyInitBody, RatchetBody, SealedBody,
    SealedContent,
};
use rekindle_crypto::{presession, CryptoError, Identity};
use rekindle_db::ratchet::{self, RatchetError};
use rekindle_db::repo::unprocessed::UnprocessedKey;
use rekindle_types::message::{ENVELOPE_FRESHNESS_WINDOW_MS, ENVELOPE_MAX_FUTURE_SKEW_MS};

use super::admission;
use super::replay::Sighting;
use crate::operations::dm::frame::{
    ad_extra, init_from_body, peer_address, ratchet_message, sealed_info, Nonce,
};
use crate::operations::friend::{state, FriendHost};

/// An opened inbound 1:1 message.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The sender's identity (hex).
    pub sender_hex: String,
    /// The message's kind: the frame's typeId, or a PreKeyInit's first
    /// payload's.
    pub type_id: DmTypeId,
    /// The sender's clock when they sent it, milliseconds.
    pub sent_at_ms: u64,
    pub body: DirectBody,
    /// Its `unprocessed` row, until the apply deletes it; `None` for a
    /// sealed frame.
    pub unprocessed: Option<UnprocessedKey>,
    /// A sealed frame's nonce, as the replay guard remembers it; `None`
    /// for a ratchet message.
    pub nonce: Option<Nonce>,
    /// The entry's seq on the sender's outbox stream (plan E2.5); 0 for a
    /// sealed frame or a frame on no stream.
    pub stream_seq: u64,
    /// The entry's seq on the sender's dm chain to us (finding 71 refined);
    /// 0 for a frame on no chain.
    pub chain_seq: u64,
}

/// Why an inbound frame did not open.
#[derive(Debug)]
pub enum Dropped {
    /// A PreKeyInit naming prekeys we no longer hold (B5), from a sender
    /// the gate admits for it: the signed `PreKeyRejected` for `to`. An
    /// `app_call` is answered with it ([`super::receive_call`]); an init
    /// that came by message gets it by message.
    Rejected {
        to: String,
        body: Box<rekindle_codec::capnp_codec::direct::DirectBody>,
    },
    /// A copy of a frame that already opened: its receipt stands.
    Duplicate,
    /// Refused (undecodable, not admitted, a bad signature, stale, or a
    /// failed decrypt): no receipt.
    Refused,
}

/// A step that refuses with no more to say.
trait OrRefused<T> {
    fn or_refused(self) -> Result<T, Dropped>;
}

impl<T> OrRefused<T> for Option<T> {
    fn or_refused(self) -> Result<T, Dropped> {
        self.ok_or(Dropped::Refused)
    }
}

impl<T, E> OrRefused<T> for Result<T, E> {
    fn or_refused(self) -> Result<T, Dropped> {
        self.map_err(|_| Dropped::Refused)
    }
}

/// Open the frame in `bytes` (after the `'D'` tag), or why (logged) it is
/// dropped.
///
/// # Errors
/// [`Dropped`]: a duplicate of a frame that opened, or refused.
pub async fn open(host: &dyn FriendHost, bytes: &[u8]) -> Result<Opened, Dropped> {
    let frame = match DmFrame::decode(bytes) {
        Ok(frame) => frame,
        Err(e) => {
            // Before admission, so the only trace of a frame lost here
            // (live A/B finding 28).
            tracing::warn!(len = bytes.len(), error = %e, "dropping a 1:1 frame that does not parse");
            return Err(Dropped::Refused);
        }
    };
    let sender_hex = hex::encode(frame.sender);
    let kind = match &frame.body {
        FrameBody::PreKeyInit(init) => init.first_type_id,
        FrameBody::Ratchet(_) | FrameBody::Sealed(_) => frame.type_id,
    };
    let init = matches!(frame.body, FrameBody::PreKeyInit(_));
    tracing::debug!(from = %sender_hex.get(..8).unwrap_or(&sender_hex), ?kind, init,
        "1:1 frame received");
    let status = admission::unblocked(host, &sender_hex).await.or_refused()?;
    if !admission::frame_admitted(host, &sender_hex, kind, status, init).await {
        return Err(Dropped::Refused);
    }
    match frame.body {
        FrameBody::Sealed(sealed) => {
            open_sealed(host, frame.sender, sender_hex, frame.type_id, &sealed)
        }
        FrameBody::Ratchet(body) => {
            open_ratchet(host, frame.sender, sender_hex, frame.type_id, &body).await
        }
        FrameBody::PreKeyInit(init) => {
            open_init(host, frame.sender, sender_hex, &init, status).await
        }
    }
}

/// A sealed frame: it opens to us ([`open_presession`]), and it was not
/// seen before.
fn open_sealed(
    host: &dyn FriendHost,
    sender: [u8; 32],
    sender_hex: String,
    type_id: DmTypeId,
    sealed: &SealedBody,
) -> Result<Opened, Dropped> {
    let identity = host.local_party().or_refused()?.identity;
    let now = host.clock().now_ms();
    let refused = |why: &str| {
        tracing::warn!(from = %sender_hex, ?type_id, why, "dropping sealed frame");
        Dropped::Refused
    };
    verify_presession(&identity, &sender, type_id, sealed).map_err(refused)?;
    // A cheap per-sender bucket before the HPKE open (plan audit G7): the
    // signature names a real sender, so one spending opens fast is the
    // only one slowed.
    if !host.stores().replay.lock().may_open(&sender, now) {
        return Err(refused("sender over its open rate"));
    }
    let content = open_verified(&identity, &sender, type_id, sealed, now).map_err(refused)?;
    match host
        .stores()
        .replay
        .lock()
        .observe(&sender, &content.nonce, content.sent_at_ms, now)
    {
        Sighting::First => {}
        Sighting::Duplicate => {
            tracing::debug!(from = %sender_hex, ?type_id, "dropping duplicate sealed frame");
            return Err(Dropped::Duplicate);
        }
        Sighting::Full => {
            tracing::warn!(from = %sender_hex, "sender over its replay quota — dropping sealed frame");
            return Err(Dropped::Refused);
        }
    }
    let Some(body) = decode_body(&sender_hex, type_id, &content.payload) else {
        forget(host, &sender, &content.nonce);
        return Err(Dropped::Refused);
    };
    Ok(Opened {
        sender_hex,
        type_id,
        sent_at_ms: content.sent_at_ms,
        stream_seq: 0,
        chain_seq: 0,
        body,
        unprocessed: None,
        nonce: Some(content.nonce),
    })
}

/// The replay guard forgets a sealed frame that opened but was not
/// applied, so its retry (same nonce) is not taken for a copy of one that
/// landed.
pub(crate) fn forget(host: &dyn FriendHost, sender: &[u8; 32], nonce: &Nonce) {
    host.stores().replay.lock().forget(sender, nonce);
}

/// The content of a sealed frame from `sender` to `identity` at `now_ms`:
/// its signature names us as the recipient and verifies under the sender,
/// it opens, and it is fresh.
///
/// # Errors
/// Why it was refused.
pub fn open_presession(
    identity: &Identity,
    sender: &[u8; 32],
    type_id: DmTypeId,
    sealed: &SealedBody,
    now_ms: u64,
) -> Result<SealedContent, &'static str> {
    verify_presession(identity, sender, type_id, sealed)?;
    open_verified(identity, sender, type_id, sealed, now_ms)
}

fn signed_seal(sealed: &SealedBody) -> presession::SignedSeal {
    presession::SignedSeal {
        enc: sealed.enc,
        ciphertext: sealed.ciphertext.clone(),
        signature: sealed.signature,
    }
}

/// Whether a sealed frame of a sealed kind is signed by `sender` for us
/// (`rekindle_crypto::presession`, before anything is decrypted).
fn verify_presession(
    identity: &Identity,
    sender: &[u8; 32],
    type_id: DmTypeId,
    sealed: &SealedBody,
) -> Result<(), &'static str> {
    sealed_info(type_id).ok_or("not a sealed kind")?;
    presession::verify(
        &identity.public_key_bytes(),
        sender,
        type_id.as_u16(),
        &signed_seal(sealed),
    )
    .map_err(presession::Refused::as_str)
}

/// The fresh content of a sealed frame whose signature verified.
fn open_verified(
    identity: &Identity,
    sender: &[u8; 32],
    type_id: DmTypeId,
    sealed: &SealedBody,
    now_ms: u64,
) -> Result<SealedContent, &'static str> {
    let info = sealed_info(type_id).ok_or("not a sealed kind")?;
    let plaintext = presession::open(
        identity,
        sender,
        type_id.as_u16(),
        info,
        &signed_seal(sealed),
    )
    .map_err(presession::Refused::as_str)?;
    let content = SealedContent::decode(&plaintext).map_err(|_| "content does not decode")?;
    if !fresh(content.sent_at_ms, now_ms) {
        return Err("stale");
    }
    Ok(content)
}

/// Whether a frame sent at `sent_at` is inside the freshness window at
/// `now`, allowing the sender's clock to run a little ahead.
fn fresh(sent_at: u64, now: u64) -> bool {
    sent_at <= now.saturating_add(ENVELOPE_MAX_FUTURE_SKEW_MS)
        && now.saturating_sub(sent_at) <= ENVELOPE_FRESHNESS_WINDOW_MS
}

/// A ratchet message under our session with the sender.
async fn open_ratchet(
    host: &dyn FriendHost,
    sender: [u8; 32],
    sender_hex: String,
    type_id: DmTypeId,
    body: &RatchetBody,
) -> Result<Opened, Dropped> {
    let local = host.local_party().or_refused()?;
    if type_id.is_ephemeral() {
        return open_ephemeral(host, sender_hex, type_id, body, &local).await;
    }
    let inbound = ratchet::decrypt_and_record_inbound(
        &host.db().or_refused()?,
        &local,
        &peer_address(&sender_hex).or_refused()?,
        ratchet_message(body),
        &ad_extra(type_id),
        type_id.as_u16(),
    )
    .await;
    match &inbound {
        // They answered under the session: our first contact landed.
        Ok(_) => crate::operations::prekeys::acknowledged(host, &sender_hex),
        // User content that did not open is asked for again after their
        // CaughtUp (plan E2.5, D24).
        Err(e) if super::retry_requests::requestable(type_id, e) => {
            super::retry_requests::note_failed(host, sender, body).await;
        }
        Err(_) => {}
    }
    recorded(host, sender, sender_hex, type_id, inbound)
}

/// An ephemeral ratchet message (`DmTypeId::is_ephemeral`: typing, a call
/// signal, DM video; plan audit G10): the ratchet advances, but nothing
/// goes to `unprocessed`, since a late one is worth nothing and is never
/// replayed.
async fn open_ephemeral(
    host: &dyn FriendHost,
    sender_hex: String,
    type_id: DmTypeId,
    body: &RatchetBody,
    local: &ratchet::LocalParty,
) -> Result<Opened, Dropped> {
    let plaintext = match ratchet::decrypt_ephemeral(
        &host.db().or_refused()?,
        local,
        &peer_address(&sender_hex).or_refused()?,
        ratchet_message(body),
        &ad_extra(type_id),
    )
    .await
    {
        Ok(plaintext) => plaintext,
        Err(RatchetError::Crypto(CryptoError::DuplicateMessage)) => {
            tracing::debug!(from = %sender_hex, ?type_id, "dropping duplicate 1:1 message");
            return Err(Dropped::Duplicate);
        }
        Err(e) => {
            tracing::warn!(from = %sender_hex, ?type_id, error = %e, "dropping 1:1 message: ratchet step failed");
            return Err(Dropped::Refused);
        }
    };
    crate::operations::prekeys::acknowledged(host, &sender_hex);
    let content = DmContent::decode(&plaintext).or_refused()?;
    let body = decode_body(&sender_hex, type_id, &content.payload).or_refused()?;
    Ok(Opened {
        sender_hex,
        type_id,
        sent_at_ms: content.sent_at_ms,
        stream_seq: content.stream_seq,
        chain_seq: content.chain_seq,
        body,
        unprocessed: None,
        nonce: None,
    })
}

/// A PreKeyInit the gate let open a session ([`admission::init_admitted`]:
/// a reset's init answers a request of ours, B11): the session it opens,
/// and its first payload.
async fn open_init(
    host: &dyn FriendHost,
    sender: [u8; 32],
    sender_hex: String,
    init: &PreKeyInitBody,
    status: admission::Status,
) -> Result<Opened, Dropped> {
    let local = host.local_party().or_refused()?;
    let accepted = ratchet::accept_init(
        &host.db().or_refused()?,
        &local,
        &peer_address(&sender_hex).or_refused()?,
        init_from_body(sender, init),
        // Only an accept of the reset we asked for may replace an
        // established session (ruling Q1; plan audit G2).
        if init.first_type_id == DmTypeId::SessionResetAccept {
            ratchet::Reinitiation::Requested
        } else {
            ratchet::Reinitiation::Refused
        },
        &ad_extra(init.first_type_id),
        init.first_type_id.as_u16(),
    )
    .await;
    let inbound = match accepted {
        Ok(accepted) => {
            crate::operations::prekeys::refilled(host, accepted.refills);
            Ok(accepted.inbound)
        }
        Err(RatchetError::ReplayedLastResortInit) => {
            // B9: a replayed last-resort init is dropped without a word.
            tracing::debug!(from = %sender_hex, "dropping a replayed last-resort PreKeyInit");
            return Err(Dropped::Refused);
        }
        Err(RatchetError::EstablishedSession) => {
            tracing::warn!(from = %sender_hex, kind = ?init.first_type_id,
                "refusing a PreKeyInit over our established session with them");
            return Err(Dropped::Refused);
        }
        Err(RatchetError::UnknownPrekey { kind, id }) => {
            tracing::info!(from = %sender_hex, %kind, id, "PreKeyInit names prekeys we no longer hold");
            return match crate::operations::prekeys::rejection(
                host,
                sender,
                &sender_hex,
                init,
                status,
            )
            .await
            {
                Some(body) => Err(Dropped::Rejected {
                    to: sender_hex,
                    body: Box::new(body),
                }),
                None => Err(Dropped::Refused),
            };
        }
        Err(e) => Err(e),
    };
    if init.first_type_id == DmTypeId::FriendAccept {
        if let Err(e) = &inbound {
            if !matches!(e, RatchetError::Crypto(CryptoError::DuplicateMessage)) {
                crate::operations::friend::inbound::on_accept_failed(
                    host,
                    &sender_hex,
                    format!("the secure-session handshake failed: {e}"),
                );
            }
        }
    }
    recorded(host, sender, sender_hex, init.first_type_id, inbound)
}

/// A decrypted message recorded in `unprocessed`, opened as `type_id`; a
/// row whose content or body does not decode is deleted (plan audit G10),
/// since no replay will ever apply it.
fn recorded(
    host: &dyn FriendHost,
    sender: [u8; 32],
    sender_hex: String,
    type_id: DmTypeId,
    inbound: Result<ratchet::Inbound, RatchetError>,
) -> Result<Opened, Dropped> {
    let inbound = match inbound {
        Ok(inbound) => inbound,
        Err(RatchetError::Crypto(CryptoError::DuplicateMessage)) => {
            tracing::debug!(from = %sender_hex, ?type_id, "dropping duplicate 1:1 message");
            return Err(Dropped::Duplicate);
        }
        Err(e) => {
            tracing::warn!(from = %sender_hex, ?type_id, error = %e, "dropping 1:1 message: ratchet step failed");
            return Err(Dropped::Refused);
        }
    };
    debug_assert_eq!(inbound.key.peer_identity, sender);
    let content = match DmContent::decode(&inbound.plaintext) {
        Ok(content) => content,
        Err(e) => {
            tracing::warn!(from = %sender_hex, error = %e, "dropping 1:1 message: content does not decode");
            delete_unprocessed(host, inbound.key);
            return Err(Dropped::Refused);
        }
    };
    let Some(body) = decode_body(&sender_hex, type_id, &content.payload) else {
        delete_unprocessed(host, inbound.key);
        return Err(Dropped::Refused);
    };
    Ok(Opened {
        sender_hex,
        type_id,
        sent_at_ms: content.sent_at_ms,
        stream_seq: content.stream_seq,
        chain_seq: content.chain_seq,
        body,
        unprocessed: Some(inbound.key),
        nonce: None,
    })
}

/// A kind's body, or `None` (logged) when the bytes are not one.
fn decode_body(sender_hex: &str, type_id: DmTypeId, payload: &[u8]) -> Option<DirectBody> {
    match DirectBody::decode(type_id, payload) {
        Ok(body) => Some(body),
        Err(e) => {
            tracing::warn!(from = %sender_hex, ?type_id, error = %e, "dropping 1:1 message: body does not decode");
            None
        }
    }
}

/// An `unprocessed` row as the message it holds, for the startup replay;
/// `None` (logged) when it no longer decodes, and the caller deletes it
/// ([`delete_unprocessed`]).
pub(crate) fn replayed(row: &rekindle_db::repo::unprocessed::Unprocessed) -> Option<Opened> {
    let sender_hex = hex::encode(row.key.peer_identity);
    let Some(type_id) = DmTypeId::from_u16(row.type_id) else {
        tracing::warn!(from = %sender_hex, type_id = row.type_id, "unprocessed row of an unknown kind");
        return None;
    };
    let content = match DmContent::decode(&row.plaintext) {
        Ok(content) => content,
        Err(e) => {
            tracing::warn!(from = %sender_hex, error = %e, "unprocessed row does not decode");
            return None;
        }
    };
    let body = decode_body(&sender_hex, type_id, &content.payload)?;
    Some(Opened {
        sender_hex,
        type_id,
        sent_at_ms: content.sent_at_ms,
        stream_seq: content.stream_seq,
        chain_seq: content.chain_seq,
        body,
        unprocessed: Some(row.key),
        nonce: None,
    })
}

/// An opened message was applied (or handed to the desktop): its
/// `unprocessed` row goes.
pub fn processed(host: &dyn FriendHost, opened: &Opened) {
    if let Some(key) = opened.unprocessed {
        delete_unprocessed(host, key);
    }
}

/// Delete the `unprocessed` row `key`.
pub(crate) fn delete_unprocessed(host: &dyn FriendHost, key: UnprocessedKey) {
    let Ok(local) = host.local_party() else {
        return;
    };
    let owner = local.owner();
    state::db_fire!(host, "delete processed message", move |conn| {
        rekindle_db::repo::unprocessed::delete(conn, &owner, &key)
    });
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
