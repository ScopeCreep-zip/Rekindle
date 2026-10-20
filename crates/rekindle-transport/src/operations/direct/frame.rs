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
use rekindle_crypto::{CryptoError, Identity};
use rekindle_db::ratchet::{self, RatchetError};
use rekindle_db::repo::unprocessed::UnprocessedKey;
use rekindle_types::message::{ENVELOPE_FRESHNESS_WINDOW_MS, ENVELOPE_MAX_FUTURE_SKEW_MS};

use super::admission;
use super::replay::Sighting;
use crate::operations::dm::frame::{
    ad_extra, init_from_body, peer_address, presession_signing_bytes, ratchet_message, sealed_aad,
    sealed_info, Nonce,
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
}

/// Why an inbound frame did not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
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
            tracing::debug!(error = %e, "dropping malformed 1:1 frame");
            return Err(Dropped::Refused);
        }
    };
    let sender_hex = hex::encode(frame.sender);
    let kind = match &frame.body {
        FrameBody::PreKeyInit(init) => init.first_type_id,
        FrameBody::Ratchet(_) | FrameBody::Sealed(_) => frame.type_id,
    };
    let status = admission::unblocked(host, &sender_hex).await.or_refused()?;
    let admitted = match &frame.body {
        FrameBody::PreKeyInit(_) => admission::init_admitted(host, &sender_hex, kind, status),
        FrameBody::Ratchet(_) | FrameBody::Sealed(_) => {
            admission::kind_admitted(&sender_hex, kind, status)
        }
    };
    if !admitted {
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
    let content = match open_presession(&identity, &sender, type_id, sealed, now) {
        Ok(content) => content,
        Err(why) => {
            tracing::warn!(from = %sender_hex, ?type_id, why, "dropping sealed frame");
            return Err(Dropped::Refused);
        }
    };
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
            tracing::warn!(from = %sender_hex, "replay guard full — dropping sealed frame");
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
    let info = sealed_info(type_id).ok_or("not a sealed kind")?;
    let me = identity.public_key_bytes();
    let signing = presession_signing_bytes(&me, type_id, &sealed.enc, &sealed.ciphertext);
    let signature = ed25519_dalek::Signature::from_bytes(&sealed.signature);
    let key = ed25519_dalek::VerifyingKey::from_bytes(sender).map_err(|_| "sender is not a key")?;
    Identity::verify(&key, &signing, &signature).map_err(|_| "bad signature")?;
    let secret = zeroize::Zeroizing::new(identity.to_x25519_secret().to_bytes());
    let plaintext = rekindle_secrets::hpke::open_base(
        &secret,
        info,
        &sealed_aad(sender, type_id),
        &sealed.enc,
        &sealed.ciphertext,
    )
    .map_err(|_| "does not open")?;
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
    let inbound = ratchet::decrypt_and_record_inbound(
        &host.db().or_refused()?,
        &local,
        &peer_address(&sender_hex).or_refused()?,
        ratchet_message(body),
        &ad_extra(type_id),
        type_id.as_u16(),
    )
    .await;
    if inbound.is_ok() {
        // They answered under the session: our first contact landed.
        crate::operations::prekeys::acknowledged(host, &sender_hex);
    }
    recorded(sender, sender_hex, type_id, inbound)
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
            crate::operations::prekeys::reject_init(host, sender, &sender_hex, init, status);
            return Err(Dropped::Refused);
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
    recorded(sender, sender_hex, init.first_type_id, inbound)
}

/// A decrypted message recorded in `unprocessed`, opened as `type_id`.
fn recorded(
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
            return Err(Dropped::Refused);
        }
    };
    let body = decode_body(&sender_hex, type_id, &content.payload).or_refused()?;
    Ok(Opened {
        sender_hex,
        type_id,
        sent_at_ms: content.sent_at_ms,
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
/// `None` (logged, and the row left to age out) when it no longer decodes.
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
        body,
        unprocessed: Some(row.key),
        nonce: None,
    })
}

/// An opened message was applied (or handed to the desktop): its
/// `unprocessed` row goes.
pub fn processed(host: &dyn FriendHost, opened: &Opened) {
    let Some(key) = opened.unprocessed else {
        return;
    };
    let Ok(local) = host.local_party() else {
        return;
    };
    let owner = local.owner();
    state::db_fire!(host, "delete processed message", move |conn| {
        rekindle_db::repo::unprocessed::delete(conn, &owner, &key)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presession(
        from: &Identity,
        to: &Identity,
        sent_at: u64,
    ) -> (DmTypeId, rekindle_codec::capnp_codec::direct::SealedBody) {
        let body = DirectBody::FriendReject;
        let frame = crate::operations::dm::frame::seal_presession(
            from,
            &to.public_key_bytes(),
            &body,
            [7; 16],
            sent_at,
        )
        .unwrap();
        let FrameBody::Sealed(sealed) = frame.body else {
            panic!("a sealed kind travels sealed");
        };
        assert_eq!(frame.sender, from.public_key_bytes());
        (frame.type_id, sealed)
    }

    #[test]
    fn a_sealed_frame_opens_to_its_recipient() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let (kind, sealed) = presession(&alice, &bob, 1_000);
        let content =
            open_presession(&bob, &alice.public_key_bytes(), kind, &sealed, 1_000).unwrap();
        assert_eq!(content.nonce, [7; 16]);
        assert_eq!(content.sent_at_ms, 1_000);
    }

    #[test]
    fn a_forged_sender_is_refused() {
        // Mallory seals a valid HPKE frame to Bob and claims Alice sent it.
        let (alice, bob, mallory) = (
            Identity::generate(),
            Identity::generate(),
            Identity::generate(),
        );
        let (kind, sealed) = presession(&mallory, &bob, 1_000);
        assert_eq!(
            open_presession(&bob, &alice.public_key_bytes(), kind, &sealed, 1_000),
            Err("bad signature")
        );
    }

    #[test]
    fn a_forged_session_reset_request_is_refused() {
        // Mallory asks Bob to reset his session with Alice, in Alice's name.
        let (alice, bob, mallory) = (
            Identity::generate(),
            Identity::generate(),
            Identity::generate(),
        );
        let body = DirectBody::SessionResetRequest(
            rekindle_codec::capnp_codec::direct::SessionResetRequest {
                reason: String::new(),
            },
        );
        let frame = crate::operations::dm::frame::seal_presession(
            &mallory,
            &bob.public_key_bytes(),
            &body,
            [5; 16],
            1_000,
        )
        .unwrap();
        let FrameBody::Sealed(sealed) = frame.body else {
            panic!("a sealed kind travels sealed");
        };
        assert_eq!(
            open_presession(
                &bob,
                &alice.public_key_bytes(),
                frame.type_id,
                &sealed,
                1_000
            ),
            Err("bad signature")
        );
    }

    #[test]
    fn a_frame_readdressed_to_someone_else_is_refused() {
        let (alice, bob, carol) = (
            Identity::generate(),
            Identity::generate(),
            Identity::generate(),
        );
        let (kind, sealed) = presession(&alice, &bob, 1_000);
        assert_eq!(
            open_presession(&carol, &alice.public_key_bytes(), kind, &sealed, 1_000),
            Err("bad signature")
        );
    }

    #[test]
    fn a_relabelled_or_stale_frame_is_refused() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let (_, sealed) = presession(&alice, &bob, 1_000);
        let from = alice.public_key_bytes();
        assert!(
            open_presession(&bob, &from, DmTypeId::SessionResetDecline, &sealed, 1_000).is_err()
        );
        assert_eq!(
            open_presession(
                &bob,
                &from,
                DmTypeId::FriendReject,
                &sealed,
                1_000 + ENVELOPE_FRESHNESS_WINDOW_MS + 1
            ),
            Err("stale")
        );
    }

    #[test]
    fn freshness_allows_skew_ahead_and_the_window_behind() {
        let now = 10_000_000;
        assert!(fresh(now, now));
        assert!(fresh(now + ENVELOPE_MAX_FUTURE_SKEW_MS, now));
        assert!(!fresh(now + ENVELOPE_MAX_FUTURE_SKEW_MS + 1, now));
        assert!(fresh(now - ENVELOPE_FRESHNESS_WINDOW_MS, now));
        assert!(!fresh(now - ENVELOPE_FRESHNESS_WINDOW_MS - 1, now));
    }
}
