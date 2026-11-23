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
};
use rekindle_crypto::CryptoError;
use rekindle_db::ratchet::{self, RatchetError};
use rekindle_db::repo::unprocessed::UnprocessedKey;

use super::admission;
use super::replay::Sighting;
pub use super::sealed_open::open_presession;
use super::sealed_open::{open_verified, verify_presession};
use crate::operations::dm::frame::{
    ad_extra, init_from_body, peer_address, ratchet_message, Nonce,
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
    /// Held until our request's accept lands ([`super::held`], plan E2.5i
    /// R2-2): user content from a `PendingOut` peer, opened on its session.
    pub held: bool,
}

/// Why an inbound frame did not open.
#[derive(Debug)]
pub enum Dropped {
    /// A PreKeyInit naming prekeys we no longer hold (B5), from a sender
    /// the gate admits for it: the signed `PreKeyRejected` for `to`,
    /// already framed and counted (framed first, D6). An `app_call` is
    /// answered with it ([`super::receive_call`]); an init that came by
    /// message gets it by message.
    Rejected { to: String, frame: Box<DmFrame> },
    /// From a sender we blocked: dropped unstored, with no state change
    /// and no event. A call is answered the silent receipt, the empty
    /// reply any reachable peer gives, so the block is never told to the
    /// blocked (LV23 review H1/H2; Signal `RecipientUtil.block` and
    /// `shouldIgnore`: nothing is sent to the blocked peer). A DM so
    /// receipted is never acked: their side shows "sent", never
    /// "delivered".
    Blocked,
    /// A copy of a frame that already opened and was applied, from `from`
    /// (as its frame claims, unauthenticated: the ratchet judges a duplicate
    /// by its header alone): its receipt stands, and an exact copy of an
    /// entry we applied is acknowledged again
    /// ([`crate::operations::dm::ack::duplicate`]; live A/B finding 73).
    /// `nonce` is a sealed frame's: its sender re-seals each attempt with a
    /// fresh `sent_at` (`dm::send::Framed::Sealed`), so its copies match by
    /// the nonce the replay guard knew it by, never by their bytes; a
    /// ratchet or PreKeyInit copy is byte-exact (`None`).
    Duplicate { from: String, nonce: Option<Nonce> },
    /// A copy of a sealed frame whose first copy is still being applied
    /// (its nonce tentative, review RV5 H1): nothing is stored yet, so no
    /// receipt; a call is answered [`super::RETRY_LATER`].
    InFlight,
    /// Refused (undecodable, blocked, not admitted, a bad signature, stale,
    /// or a failed decrypt): no receipt, a call is answered NAK.
    Refused,
    /// Not admitted yet (real-app finding GDI1,
    /// [`admission::not_yet`]): the gate refused it from a sender we asked
    /// (`PendingOut`), whose accept may still be on its way. Nothing is
    /// opened or stored; a call is answered [`super::RETRY_LATER`], so the
    /// sender's receipted row is attempted again after the accept lands;
    /// a chain entry stays pending.
    NotYet,
    /// Not judged: an internal error of ours (no unlocked identity, our
    /// database, the block-list or friendship read, or framing our reply
    /// failed). Not a refusal: a call is answered [`super::RETRY_LATER`],
    /// so the caller attempts it again (RV2, `c37fef40`; RV5 R2, M2).
    Failed,
}

/// A step that refuses with no more to say.
trait OrRefused<T> {
    fn or_refused(self) -> Result<T, Dropped>;
    /// [`Dropped::Failed`]: the step could not run, nothing was judged.
    fn or_failed(self) -> Result<T, Dropped>;
}

impl<T> OrRefused<T> for Option<T> {
    fn or_refused(self) -> Result<T, Dropped> {
        self.ok_or(Dropped::Refused)
    }

    fn or_failed(self) -> Result<T, Dropped> {
        self.ok_or(Dropped::Failed)
    }
}

impl<T, E> OrRefused<T> for Result<T, E> {
    fn or_refused(self) -> Result<T, Dropped> {
        self.map_err(|_| Dropped::Refused)
    }

    fn or_failed(self) -> Result<T, Dropped> {
        self.map_err(|_| Dropped::Failed)
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
    // A PreKeyInit for a session we hold is a message on it (plan E2.5h:
    // the peer wraps every message until we answer), admitted as its kind
    // is, not as a session opening.
    let init = match &frame.body {
        FrameBody::PreKeyInit(body) => !known_init(host, &sender_hex, body.base_key).await?,
        FrameBody::Ratchet(_) | FrameBody::Sealed(_) => false,
    };
    // The full key: the log scrubber turns it into a per-process tag (B19).
    tracing::debug!(from = %sender_hex, ?kind, init, "1:1 frame received");
    let status = admission::unblocked(host, &sender_hex)
        .await
        .map_err(|barred| match barred {
            admission::Barred::Blocked => Dropped::Blocked,
            admission::Barred::Unreadable => Dropped::Failed,
        })?;
    // User content from a peer we asked, ahead of their accept, opens on
    // its session and is held (plan E2.5i R2-2); a sealed one never is.
    let hold = super::held::holds(kind, status) && !matches!(frame.body, FrameBody::Sealed(_));
    if !hold && !admission::frame_admitted(host, &sender_hex, kind, status, init).await {
        return Err(if admission::not_yet(status) {
            Dropped::NotYet
        } else {
            Dropped::Refused
        });
    }
    let mut opened = match frame.body {
        FrameBody::Sealed(sealed) => {
            open_sealed(host, frame.sender, sender_hex, frame.type_id, &sealed)
        }
        FrameBody::Ratchet(body) => {
            open_ratchet(host, frame.sender, sender_hex, frame.type_id, &body).await
        }
        FrameBody::PreKeyInit(init) => {
            open_init(host, frame.sender, sender_hex, &init, status).await
        }
    }?;
    opened.held = hold;
    Ok(opened)
}

/// Whether a session we hold with `sender_hex` has `base_key`.
async fn known_init(
    host: &dyn FriendHost,
    sender_hex: &str,
    base_key: [u8; 32],
) -> Result<bool, Dropped> {
    ratchet::is_known_init(
        &host.db().or_failed()?,
        &host.local_party().or_failed()?.owner(),
        &peer_address(sender_hex).or_refused()?,
        base_key,
    )
    .await
    .or_failed()
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
    let identity = host.local_party().or_failed()?.identity;
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
            return Err(Dropped::Duplicate {
                from: sender_hex.clone(),
                nonce: Some(content.nonce),
            });
        }
        Sighting::InFlight => {
            tracing::debug!(from = %sender_hex, ?type_id, "sealed frame's first copy in flight");
            return Err(Dropped::InFlight);
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
    crate::operations::dm::route_health::heard(host, &sender_hex);
    Ok(Opened {
        sender_hex,
        type_id,
        sent_at_ms: content.sent_at_ms,
        stream_seq: 0,
        chain_seq: 0,
        body,
        unprocessed: None,
        nonce: Some(content.nonce),
        held: false,
    })
}

/// The replay guard forgets a sealed frame that opened but was not
/// applied, so its retry (same nonce) is not taken for a copy of one that
/// landed.
pub(crate) fn forget(host: &dyn FriendHost, sender: &[u8; 32], nonce: &Nonce) {
    host.stores().replay.lock().forget(sender, nonce);
}

/// Settle a sealed frame's nonce: its apply is done (stored, or judged),
/// so a copy from now on is a duplicate (review RV5 H1).
fn settle(host: &dyn FriendHost, opened: &Opened) {
    let Some(nonce) = opened.nonce else {
        return;
    };
    let Ok(Ok(sender)) = hex::decode(&opened.sender_hex).map(<[u8; 32]>::try_from) else {
        return;
    };
    host.stores().replay.lock().settle(&sender, &nonce);
}

/// A ratchet message under our session with the sender.
async fn open_ratchet(
    host: &dyn FriendHost,
    sender: [u8; 32],
    sender_hex: String,
    type_id: DmTypeId,
    body: &RatchetBody,
) -> Result<Opened, Dropped> {
    let local = host.local_party().or_failed()?;
    if type_id.is_ephemeral() {
        return open_ephemeral(host, sender_hex, type_id, body, &local).await;
    }
    let inbound = ratchet::decrypt_and_record_inbound(
        &host.db().or_failed()?,
        &local,
        &peer_address(&sender_hex).or_refused()?,
        ratchet_message(body),
        &ad_extra(type_id),
        type_id.as_u16(),
    )
    .await;
    match &inbound {
        // They answered under the session: our first contact landed.
        Ok(_) => {
            crate::operations::prekeys::acknowledged(host, &sender_hex);
            crate::operations::dm::route_health::heard(host, &sender_hex);
            crate::operations::dm::handshake::settle_first_contact(host, &sender_hex).await;
        }
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
    let ephemeral = match ratchet::decrypt_ephemeral(
        &host.db().or_failed()?,
        local,
        &peer_address(&sender_hex).or_refused()?,
        ratchet_message(body),
        &ad_extra(type_id),
    )
    .await
    {
        Ok(ephemeral) => ephemeral,
        Err(RatchetError::Crypto(CryptoError::DuplicateMessage)) => {
            tracing::debug!(from = %sender_hex, ?type_id, "dropping duplicate 1:1 message");
            return Err(Dropped::Duplicate {
                from: sender_hex.clone(),
                nonce: None,
            });
        }
        Err(e) => {
            tracing::warn!(from = %sender_hex, ?type_id, error = %e, "dropping 1:1 message: ratchet step failed");
            return Err(Dropped::Refused);
        }
    };
    crate::operations::dm::route_health::heard(host, &sender_hex);
    crate::operations::prekeys::acknowledged(host, &sender_hex);
    if ephemeral.ended_session {
        crate::operations::prekeys::session_ended(host, &sender_hex);
    }
    let content = DmContent::decode(&ephemeral.plaintext).or_refused()?;
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
        held: false,
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
    let local = host.local_party().or_failed()?;
    let accepted = ratchet::accept_init(
        &host.db().or_failed()?,
        &local,
        &peer_address(&sender_hex).or_refused()?,
        init_from_body(sender, init),
        // Only an accept of the reset we asked for, or of the friendship
        // we asked for, may replace an established session (ruling Q1;
        // plan audit G2; plan E2.5h).
        match init.first_type_id {
            DmTypeId::SessionResetAccept => ratchet::Reinitiation::Requested,
            DmTypeId::FriendAccept => ratchet::Reinitiation::Befriended,
            _ => ratchet::Reinitiation::Refused,
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
                Ok(Some(frame)) => Err(Dropped::Rejected {
                    to: sender_hex,
                    frame: Box::new(frame),
                }),
                Ok(None) => Err(Dropped::Refused),
                Err(crate::operations::prekeys::Unjudged) => Err(Dropped::Failed),
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

/// What a failed ratchet step is: our database failing is no judgement
/// ([`Dropped::Failed`], the call is answered retry later, RV5 R2, M2); any other
/// failure is the frame's ([`Dropped::Refused`]).
pub(crate) fn ratchet_dropped(error: &RatchetError) -> Dropped {
    match error {
        RatchetError::Db(_) => Dropped::Failed,
        _ => Dropped::Refused,
    }
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
            return Err(Dropped::Duplicate {
                from: sender_hex.clone(),
                nonce: None,
            });
        }
        Err(e) => {
            tracing::warn!(from = %sender_hex, ?type_id, error = %e, "dropping 1:1 message: ratchet step failed");
            return Err(ratchet_dropped(&e));
        }
    };
    debug_assert_eq!(inbound.key.peer_identity, sender);
    if inbound.ended_session {
        crate::operations::prekeys::session_ended(host, &sender_hex);
    }
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
        held: false,
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
        held: false,
    })
}

/// An opened message was applied, judged or handed to the desktop: its
/// `unprocessed` row goes, and a sealed frame's nonce settles (RV5 H1).
pub fn processed(host: &dyn FriendHost, opened: &Opened) {
    settle(host, opened);
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
