//! The 1:1 frames on both hosts (plan E2.3): every 1:1 datagram is
//! `'D' ‖ DmFrame`, and a kind travels as its typeId's carrier says:
//!
//! - a ratchet message under the peer's session (`rekindle_db::ratchet`,
//!   recorded before anything is sent);
//! - a PreKeyInit that opens a session and carries its first payload;
//! - a pre-session frame, HPKE Base-sealed to the peer's identity and
//!   signed by ours over `DM_PRESESSION_SIG ‖ recipient ‖ typeId ‖ enc ‖
//!   ct` (RFC 9180 §9.1.1's mitigation: Base mode authenticates no
//!   sender, and Veilid names none).
//!
//! The typeId rides in the clear, so every ratchet AD binds it
//! ([`ad_extra`]).

use rekindle_codec::capnp_codec::direct::{
    Carrier, ContentHint, DirectBody, DmContent, DmFrame, DmTypeId, FrameBody, PqPrekey,
    PreKeyInitBody, RatchetBody, RatchetHeader, SealedBody, SealedContent,
};
use rekindle_crypto::signal::{Header, PqPrekeyId, PreKeyBundle, PreKeyInit, RatchetMessage};
use rekindle_crypto::{presession, CryptoError, Identity};
use rekindle_db::ratchet::{self, PeerAddress, RatchetError};
use rekindle_types::domains;

use super::DIRECT_TAG;
use crate::operations::friend::{FriendError, FriendHost};

/// A sealed frame's random nonce: the replay guard keeps `(sender, nonce)`.
pub type Nonce = [u8; 16];

/// The ratchet AD's `ad_extra` for a kind: its typeId, so a frame cannot
/// be relabelled as another kind.
#[must_use]
pub fn ad_extra(type_id: DmTypeId) -> [u8; 2] {
    type_id.as_u16().to_be_bytes()
}

/// Whether a ratchet message of `type_id` is user content, logged in
/// `message_send_log` when we send it and asked for again with a
/// `RetryRequest` when it does not open (plan E2.5, D24): a direct message
/// and its edits, deletes and reactions.
#[must_use]
pub fn is_logged(type_id: DmTypeId) -> bool {
    matches!(
        type_id,
        DmTypeId::DmText | DmTypeId::DmEdit | DmTypeId::DmDelete | DmTypeId::DmReaction
    )
}

/// The HPKE info label of a sealed kind; `None` for the other carriers.
#[must_use]
pub fn sealed_info(type_id: DmTypeId) -> Option<&'static str> {
    Some(match type_id {
        DmTypeId::FriendRequest => domains::DM_FRIEND_REQUEST_HPKE,
        DmTypeId::PreKeyRejected => domains::DM_PREKEY_REJECTED_HPKE,
        DmTypeId::FriendReject => domains::DM_FRIEND_REJECT_HPKE,
        DmTypeId::SessionResetRequest => domains::DM_SESSION_RESET_REQUEST_HPKE,
        DmTypeId::SessionResetDecline => domains::DM_SESSION_RESET_DECLINE_HPKE,
        DmTypeId::RegisterPushRelay => domains::DM_PUSH_REGISTER_HPKE,
        DmTypeId::UnregisterPushRelay => domains::DM_PUSH_UNREGISTER_HPKE,
        DmTypeId::WakeNotify => domains::DM_PUSH_WAKE_HPKE,
        _ => return None,
    })
}

/// The datagram for `frame`: [`DIRECT_TAG`] then its encoding. Every 1:1
/// send builds its bytes here.
///
/// # Errors
/// `Invalid` when the datagram is over Veilid's message cap: it fails on
/// every path, so it is refused rather than queued (plan C7.25).
pub fn wire(frame: &DmFrame) -> Result<Vec<u8>, FriendError> {
    let encoded = frame.encode();
    let mut datagram = Vec::with_capacity(encoded.len() + 1);
    datagram.push(DIRECT_TAG);
    datagram.extend_from_slice(&encoded);
    rekindle_records::port::check_message_len(&datagram)
        .map_err(|e| FriendError::Invalid(e.to_string()))?;
    Ok(datagram)
}

/// The peer a frame to `to` (a hex identity) is for.
///
/// # Errors
/// `Invalid` when `to` is not a public key.
pub fn peer_address(to: &str) -> Result<PeerAddress, FriendError> {
    super::recipient_key(to).map(PeerAddress::sole_device)
}

/// A ratchet step's failure as the friend operations report it.
pub(crate) fn ratchet_error(to: &str, e: RatchetError) -> FriendError {
    match e {
        RatchetError::Crypto(CryptoError::NoSession(_)) => FriendError::Conflict(format!(
            "No secure session with {to}. They haven't completed a handshake yet — they need \
             to accept your friend request before you can send encrypted messages."
        )),
        RatchetError::Crypto(CryptoError::IdentityAddressMismatch { .. }) => {
            FriendError::Invalid(format!("the bundle for {to} is under another identity"))
        }
        other => FriendError::Failed(format!("ratchet step with {to}: {other}")),
    }
}

/// The plaintext of a ratchet or PreKeyInit message carrying `body`.
fn content(host: &dyn FriendHost, body: &DirectBody, hint: ContentHint) -> Vec<u8> {
    DmContent {
        content_hint: hint,
        sent_at_ms: host.clock().now_ms(),
        payload: body.encode(),
    }
    .encode()
}

/// `body` as a ratchet message under our session with `to`, recorded
/// before it is returned (user content with its send-log row, [`is_logged`]). With no session at all with an accepted friend
/// (they rotated their identity, or ours moved), the message opens one from
/// a slot of their prekey record (plan E2.3: "the next send to the new
/// identity initiates from its published bundle"; E2.4: from its record). This never re-initiates
/// a session that exists: a broken one is ruling Q1's prompt (E2.5). Typing
/// never opens one (plan audit R-4): with no session it is refused.
///
/// # Errors
/// No session with `to` and none can be opened (`Conflict`), or the step
/// failed.
pub async fn ratchet_frame(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
    hint: ContentHint,
) -> Result<DmFrame, FriendError> {
    let type_id = body.type_id();
    let local = host.local_party()?;
    let peer = peer_address(to)?;
    let (db, plaintext) = (host.db()?, content(host, body, hint));
    let encrypted = if is_logged(type_id) {
        ratchet::encrypt_record_and_log_outbound(
            &db,
            &local,
            &peer,
            &plaintext,
            &ad_extra(type_id),
            type_id.as_u16(),
        )
        .await
    } else {
        ratchet::encrypt_and_record_outbound(&db, &local, &peer, &plaintext, &ad_extra(type_id))
            .await
    };
    let message = match encrypted {
        Ok(message) => message,
        Err(RatchetError::Crypto(CryptoError::NoSession(_)))
            if type_id != DmTypeId::Typing
                && crate::operations::friend::state::is_accepted(host, to) =>
        {
            let bundle = crate::operations::prekeys::bundle_for_friend(host, to).await?;
            return initiate_frame(host, to, bundle, body).await;
        }
        Err(e) => return Err(ratchet_error(to, e)),
    };
    Ok(DmFrame {
        type_id,
        sender: local.owner(),
        body: FrameBody::Ratchet(ratchet_body(message)),
    })
}

/// Open a session with `to` from its `bundle`, carrying `first` (an accept,
/// or any ratchet kind) as the init's encrypted first payload.
///
/// # Errors
/// `first` may not open a session, the bundle is not `to`'s, or the step
/// failed.
pub async fn initiate_frame(
    host: &dyn FriendHost,
    to: &str,
    bundle: PreKeyBundle,
    first: &DirectBody,
) -> Result<DmFrame, FriendError> {
    let first_type_id = first.type_id();
    if !first_type_id.is_first_payload() {
        return Err(FriendError::Invalid(format!(
            "{first_type_id:?} cannot open a session"
        )));
    }
    let local = host.local_party()?;
    let peer = peer_address(to)?;
    let init = ratchet::initiate_and_record_outbound(
        &host.db()?,
        &local,
        &peer,
        bundle,
        &content(host, first, ContentHint::Default),
        &ad_extra(first_type_id),
    )
    .await
    .map_err(|e| ratchet_error(to, e))?;
    crate::operations::prekeys::began_first_contact(host, to, first_type_id);
    Ok(DmFrame {
        type_id: DmTypeId::PreKeyInit,
        sender: local.owner(),
        body: FrameBody::PreKeyInit(init_body(init, first_type_id)),
    })
}

/// `body` (a sealed kind) as a pre-session frame to `to` under `nonce`.
/// A retry passes its first attempt's nonce, so a copy that already
/// arrived is dropped; `sent_at` is always now.
///
/// # Errors
/// `body` is not a sealed kind, `to` is not a key, or the identity is
/// locked.
pub fn sealed_frame(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
    nonce: Nonce,
) -> Result<DmFrame, FriendError> {
    let identity = host.local_party()?.identity;
    let recipient = super::recipient_key(to)?;
    seal_presession(&identity, &recipient, body, nonce, host.clock().now_ms())
}

/// [`sealed_frame`] from `identity` to `recipient` at `now_ms`.
///
/// # Errors
/// `body` is not a sealed kind, or `recipient` is not a curve point.
pub fn seal_presession(
    identity: &Identity,
    recipient: &[u8; 32],
    body: &DirectBody,
    nonce: Nonce,
    now_ms: u64,
) -> Result<DmFrame, FriendError> {
    let type_id = body.type_id();
    let info = sealed_info(type_id)
        .ok_or_else(|| FriendError::Invalid(format!("{type_id:?} is not a sealed kind")))?;
    let plaintext = SealedContent {
        nonce,
        sent_at_ms: now_ms,
        payload: body.encode(),
    }
    .encode();
    let sealed = presession::seal(identity, recipient, type_id.as_u16(), info, &plaintext)
        .map_err(|e| FriendError::Invalid(format!("seal {type_id:?}: {e}")))?;
    Ok(DmFrame {
        type_id,
        sender: identity.public_key_bytes(),
        body: FrameBody::Sealed(SealedBody {
            enc: sealed.enc,
            ciphertext: sealed.ciphertext,
            signature: sealed.signature,
        }),
    })
}

/// `body` to `to` as its kind travels after a session exists: a ratchet
/// message, or a sealed frame under a fresh nonce.
///
/// # Errors
/// A PreKeyInit or accept kind (they open a session: [`initiate_frame`]),
/// or the sealing failed.
pub async fn frame(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
    hint: ContentHint,
) -> Result<DmFrame, FriendError> {
    match body.type_id().carrier() {
        Carrier::Ratchet => ratchet_frame(host, to, body, hint).await,
        Carrier::Sealed => {
            let mut nonce = Nonce::default();
            host.rng().fill(&mut nonce);
            sealed_frame(host, to, body, nonce)
        }
        Carrier::PreKeyInit | Carrier::FirstPayload => Err(FriendError::Invalid(format!(
            "{:?} opens a session: initiate it from a bundle",
            body.type_id()
        ))),
    }
}

/// A ratchet message's wire body.
#[must_use]
pub fn ratchet_body(message: RatchetMessage) -> RatchetBody {
    RatchetBody {
        header: RatchetHeader {
            dh: message.header.dh,
            pn: message.header.pn,
            n: message.header.n,
        },
        ciphertext: message.ciphertext,
    }
}

/// The ratchet message a wire body carries.
#[must_use]
pub fn ratchet_message(body: &RatchetBody) -> RatchetMessage {
    RatchetMessage {
        header: Header {
            dh: body.header.dh,
            pn: body.header.pn,
            n: body.header.n,
        },
        ciphertext: body.ciphertext.clone(),
    }
}

/// A PreKeyInit's wire body; `IK_A` travels as the frame's sender.
#[must_use]
pub fn init_body(init: PreKeyInit, first_type_id: DmTypeId) -> PreKeyInitBody {
    PreKeyInitBody {
        base_key: init.base_key,
        signed_prekey_id: init.signed_prekey_id,
        one_time_prekey_id: init.one_time_prekey_id,
        pq_prekey: match init.pq_prekey {
            PqPrekeyId::OneTime(id) => PqPrekey::OneTime(id),
            PqPrekeyId::LastResort(id) => PqPrekey::LastResort(id),
        },
        kem_ciphertext: init.kem_ciphertext,
        first_type_id,
        message: ratchet_body(init.message),
    }
}

/// The PreKeyInit a wire body from `sender` carries.
#[must_use]
pub fn init_from_body(sender: [u8; 32], body: &PreKeyInitBody) -> PreKeyInit {
    PreKeyInit {
        identity_key: sender,
        base_key: body.base_key,
        signed_prekey_id: body.signed_prekey_id,
        one_time_prekey_id: body.one_time_prekey_id,
        pq_prekey: match body.pq_prekey {
            PqPrekey::OneTime(id) => PqPrekeyId::OneTime(id),
            PqPrekey::LastResort(id) => PqPrekeyId::LastResort(id),
        },
        kem_ciphertext: body.kem_ciphertext.clone(),
        message: ratchet_message(&body.message),
    }
}
