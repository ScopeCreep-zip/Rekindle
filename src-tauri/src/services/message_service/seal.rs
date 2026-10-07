//! Sealing and signing of outgoing 1:1 envelopes.
//!
//! Every send path (direct, `app_call`, queued, retried) builds its
//! envelope here, so the payload's [`Sealing`] is applied in one place
//! and every signature is bound to the recipient.

use std::sync::Arc;

use rand::RngCore as _;
use rekindle_codec::message::envelope::{MessageEnvelope, MessagePayload, Sealing};
use rekindle_protocol::messaging::sender::build_envelope_from_secret;

use crate::state::AppState;

/// The recipient's identity key as signed into the envelope.
pub(super) fn recipient_key(to: &str) -> Result<[u8; 32], String> {
    rekindle_types::key_format::public_key_hex(to)
        .map(|k| k.to_bytes())
        .map_err(|e| format!("recipient key {to}: {e}"))
}

/// Serialize `payload` and seal it as its [`Sealing`] requires. A
/// `Session` payload with no session fails; it is never sent in plain.
async fn seal(
    state: &Arc<AppState>,
    to: &str,
    payload: &MessagePayload,
) -> Result<Vec<u8>, String> {
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|e| format!("serialize payload: {e}"))?;
    match payload.sealing() {
        Sealing::Plain => Ok(payload_bytes),
        Sealing::Session => {
            // Clone the Arc out so the guard is not held across `.await`.
            let handle = state
                .signal_manager
                .read()
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| {
                    format!(
                        "No Signal manager — cannot send encrypted message to {to}. \
                         Sign in to initialize Signal sessions."
                    )
                })?;
            match handle.manager.has_session(to) {
                Ok(true) => handle
                    .manager
                    .encrypt(to, &payload_bytes)
                    .await
                    .map_err(|e| format!("Signal encrypt failed for {to}: {e}")),
                Ok(false) => Err(format!(
                    "No secure session with {to}. They haven't completed a Signal handshake yet — \
                     they need to accept your friend request before you can send encrypted messages. \
                     Verify their safety number out-of-band before resuming sensitive conversation."
                )),
                Err(e) => Err(format!(
                    "Signal session check failed for {to}: {e}. \
                     The session may be corrupted; re-establish from Friend → Reset Secure Session \
                     after verifying their safety number out-of-band."
                )),
            }
        }
    }
}

/// Sign already-sealed bytes to `to`, timestamped now.
fn sign(
    state: &Arc<AppState>,
    to: &str,
    nonce: Vec<u8>,
    sealed: Vec<u8>,
) -> Result<MessageEnvelope, String> {
    let recipient = recipient_key(to)?;
    let secret_key = {
        let sk = state.identity_secret.lock();
        *sk.as_ref().ok_or("signing key not initialized")?
    };
    Ok(build_envelope_from_secret(
        &secret_key,
        &recipient,
        rekindle_utils::timestamp_ms(),
        nonce,
        sealed,
    ))
}

/// A sealed, signed envelope carrying `payload` to `to`, under a fresh
/// random nonce.
pub(super) async fn build_signed_envelope(
    state: &Arc<AppState>,
    to: &str,
    payload: &MessagePayload,
) -> Result<MessageEnvelope, String> {
    let sealed = seal(state, to, payload).await?;
    let mut nonce = vec![0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce);
    sign(state, to, nonce, sealed)
}

/// Re-sign a queued envelope for another delivery attempt: same nonce and
/// sealed payload, fresh timestamp. The receiver drops it if an earlier
/// attempt already arrived.
pub(crate) fn resign_for_retry(
    state: &Arc<AppState>,
    to: &str,
    envelope: &MessageEnvelope,
) -> Result<MessageEnvelope, String> {
    sign(state, to, envelope.nonce.clone(), envelope.payload.clone())
}
