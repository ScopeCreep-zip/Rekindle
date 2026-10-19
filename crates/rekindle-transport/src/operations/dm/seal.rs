//! Sealing and signing of outgoing 1:1 envelopes.
//!
//! Every send path (direct, `app_call`, queued, retried) builds its
//! envelope here, so the payload's [`Sealing`] is applied in one place
//! and every signature is bound to the recipient.

use rekindle_codec::message::envelope::{MessageEnvelope, MessagePayload, Sealing};
use rekindle_protocol::messaging::sender::build_envelope_from_secret;

use crate::operations::friend::{FriendError, FriendHost};

/// The envelope nonce's length.
const NONCE_LEN: usize = 16;

/// The recipient's identity key as signed into the envelope.
///
/// # Errors
/// `Invalid` when `to` is not a public key.
pub(crate) fn recipient_key(to: &str) -> Result<[u8; 32], FriendError> {
    rekindle_types::key_format::public_key_hex(to)
        .map(|k| k.to_bytes())
        .map_err(|e| FriendError::Invalid(format!("recipient key {to}: {e}")))
}

/// Serialize `payload` and seal it as its [`Sealing`] requires. A
/// `Session` payload with no session fails; it is never sent in plain.
async fn seal(
    host: &dyn FriendHost,
    to: &str,
    payload: &MessagePayload,
) -> Result<Vec<u8>, FriendError> {
    let payload_bytes = serde_json::to_vec(payload)
        .map_err(|e| FriendError::Failed(format!("serialize payload: {e}")))?;
    match payload.sealing() {
        Sealing::Plain => Ok(payload_bytes),
        Sealing::Session => {
            let signal = host.signal()?;
            match signal.has_session(to) {
                Ok(true) => signal
                    .encrypt(to, &payload_bytes)
                    .await
                    .map_err(|e| FriendError::Failed(format!("Signal encrypt failed for {to}: {e}"))),
                Ok(false) => Err(FriendError::Conflict(format!(
                    "No secure session with {to}. They haven't completed a Signal handshake yet — \
                     they need to accept your friend request before you can send encrypted messages. \
                     Verify their safety number out-of-band before resuming sensitive conversation."
                ))),
                Err(e) => Err(FriendError::Failed(format!(
                    "Signal session check failed for {to}: {e}. \
                     The session may be corrupted; re-establish from Friend → Reset Secure Session \
                     after verifying their safety number out-of-band."
                ))),
            }
        }
    }
}

/// Sign already-sealed bytes to `to`, timestamped now.
fn sign(
    host: &dyn FriendHost,
    to: &str,
    nonce: Vec<u8>,
    sealed: Vec<u8>,
) -> Result<MessageEnvelope, FriendError> {
    let recipient = recipient_key(to)?;
    let secret = host.identity_secret()?;
    Ok(build_envelope_from_secret(
        &secret,
        &recipient,
        host.clock().now_ms(),
        nonce,
        sealed,
    ))
}

/// A sealed, signed envelope carrying `payload` to `to`, under a fresh
/// random nonce.
///
/// # Errors
/// The payload could not be sealed (no session for a `Session` payload),
/// or the identity is locked.
pub(crate) async fn build_signed_envelope(
    host: &dyn FriendHost,
    to: &str,
    payload: &MessagePayload,
) -> Result<MessageEnvelope, FriendError> {
    let sealed = seal(host, to, payload).await?;
    let mut nonce = vec![0u8; NONCE_LEN];
    host.rng().fill(&mut nonce);
    sign(host, to, nonce, sealed)
}

/// Re-sign a queued envelope for another delivery attempt: same nonce and
/// sealed payload, fresh timestamp. The receiver drops it if an earlier
/// attempt already arrived.
///
/// # Errors
/// The identity is locked, or `to` is not a public key.
pub fn resign_for_retry(
    host: &dyn FriendHost,
    to: &str,
    envelope: &MessageEnvelope,
) -> Result<MessageEnvelope, FriendError> {
    sign(host, to, envelope.nonce.clone(), envelope.payload.clone())
}
