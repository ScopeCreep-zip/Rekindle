//! Signed invite blobs: creation, verification, recency policy, URLs.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

///
/// Encoded as JSON, signed with Ed25519, then base64url-encoded for sharing
/// as a `rekindle://` URL or plain string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteBlob {
    /// Sender's Ed25519 public key (hex).
    pub public_key: String,
    /// Sender's display name.
    pub display_name: String,
    /// Sender's mailbox DHT record key (for reading route blob).
    pub mailbox_dht_key: String,
    /// Sender's private profile DHT record key (for presence watching).
    pub profile_dht_key: String,
    /// The invite's single-use request inbox (plan E2.5, D17): the
    /// requester writes its sealed `FriendRequest` there, so the invite
    /// carries no route.
    pub inbox: InboxCapability,
    /// Correlation token linking this invite to tracked outgoing invites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite_id: Option<String>,
    /// Unix epoch milliseconds when the invite was minted (B11 hardening).
    /// Recipients enforce a max-age policy via [`check_invite_recency`] so
    /// leaked / harvested links can't be redeemed indefinitely. Older
    /// invites (pre-issued_at-field) decode with `0` here and are rejected
    /// as stale by [`check_invite_recency`] — no legacy fallback.
    #[serde(default)]
    pub issued_at: u64,
    /// Ed25519 signature over `FRIEND_INVITE_SIG ‖` the JSON of all fields
    /// above.
    #[serde(with = "rekindle_types::base64_bytes")]
    pub signature: Vec<u8>,
}

/// A friend invite's request inbox: the `SMPL(o_cnt: 1, [oneTimeWriter])`
/// record (`rekindle_types::dht_layout::request_inbox`) and the keypair
/// text of its one-time writer, the capability to write it once. The
/// writer's secret is wiped when the value is dropped.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxCapability {
    /// The inbox's record key.
    pub record_key: String,
    /// The one-time writer's keypair text (`VLD0:<public>:<secret>`).
    pub writer: String,
}

impl Drop for InboxCapability {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.writer);
    }
}

impl std::fmt::Debug for InboxCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboxCapability")
            .field("record_key", &self.record_key)
            .field("writer", &"<redacted>")
            .finish()
    }
}

/// Create a signed invite blob from identity credentials.
///
/// Signs over a JSON-serialized form of the invite data (excluding the
/// signature field itself) using the Ed25519 secret key. `issued_at_ms` is
/// covered by the signature so a third party cannot back-date a leaked
/// blob to extend its useful life past the recipient's recency window.
pub fn create_invite_blob(
    secret_key: &[u8; 32],
    public_key: &str,
    display_name: &str,
    mailbox_dht_key: &str,
    profile_dht_key: &str,
    inbox: &InboxCapability,
    invite_id: Option<&str>,
    issued_at_ms: u64,
) -> InviteBlob {
    use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};

    let signing_key = SigningKey::from_bytes(secret_key);

    let signature = signing_key.sign(&signable_bytes(
        public_key,
        display_name,
        mailbox_dht_key,
        profile_dht_key,
        inbox,
        invite_id,
        issued_at_ms,
    ));

    InviteBlob {
        public_key: public_key.to_string(),
        display_name: display_name.to_string(),
        mailbox_dht_key: mailbox_dht_key.to_string(),
        profile_dht_key: profile_dht_key.to_string(),
        inbox: inbox.clone(),
        invite_id: invite_id.map(str::to_string),
        issued_at: issued_at_ms,
        signature: signature.to_bytes().to_vec(),
    }
}

/// The bytes an invite's signature covers: the registered domain
/// `FRIEND_INVITE_SIG`, then every field but the signature as JSON, with
/// the inbox as its record key and writer text. The domain keeps
/// an identity-key signature over invite JSON from ever verifying as any
/// other identity-key signature, and the reverse.
/// The invite carries no prekey bundle: B1's accepter starts the
/// handshake from the FriendRequest's own bundle, and nothing read the
/// invite's copy, which was ~75 % of the URL (plan C7.25).
fn signable_bytes(
    public_key: &str,
    display_name: &str,
    mailbox_dht_key: &str,
    profile_dht_key: &str,
    inbox: &InboxCapability,
    invite_id: Option<&str>,
    issued_at_ms: u64,
) -> Vec<u8> {
    let signable = serde_json::json!({
        "public_key": public_key,
        "display_name": display_name,
        "mailbox_dht_key": mailbox_dht_key,
        "profile_dht_key": profile_dht_key,
        "inbox_key": inbox.record_key,
        "inbox_writer": inbox.writer,
        "invite_id": invite_id,
        "issued_at": issued_at_ms,
    });
    let json = serde_json::to_vec(&signable).unwrap_or_default();
    let domain = rekindle_types::domains::FRIEND_INVITE_SIG.as_bytes();
    let mut bytes = Vec::with_capacity(domain.len() + json.len());
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&json);
    bytes
}

/// Verify the Ed25519 signature on an invite blob.
///
/// Returns `Ok(())` if the signature is valid, `Err` otherwise.
pub fn verify_invite_blob(blob: &InviteBlob) -> Result<(), String> {
    use rekindle_secrets::ed25519_dalek::{Signature, VerifyingKey};

    let pub_bytes =
        hex::decode(&blob.public_key).map_err(|e| format!("invalid public key hex: {e}"))?;
    let pub_array: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;
    let verifying_key =
        VerifyingKey::from_bytes(&pub_array).map_err(|e| format!("invalid public key: {e}"))?;

    let sig_array: [u8; 64] = blob
        .signature
        .clone()
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;
    let signature = Signature::from_bytes(&sig_array);

    let signed = signable_bytes(
        &blob.public_key,
        &blob.display_name,
        &blob.mailbox_dht_key,
        &blob.profile_dht_key,
        &blob.inbox,
        blob.invite_id.as_deref(),
        blob.issued_at,
    );

    verifying_key
        .verify_strict(&signed, &signature)
        .map_err(|e| {
            // Most signature failures users hit in practice are version
            // mismatches: the sender's build pre-dates the B11 issued_at
            // field, so the canonical bytes the receiver reconstructs
            // (which always include `"issued_at": 0`) don't match what
            // the sender signed. Surface this hint instead of just the
            // raw cryptographic error so the user knows what to do.
            if blob.issued_at == 0 {
                format!(
                    "invite link is from an older app version (no issued_at). \
                     Ask the sender to regenerate the invite on a current build. \
                     (raw: {e})"
                )
            } else {
                format!("invalid invite signature: {e}")
            }
        })
}

/// Reject an invite that was minted more than `max_age_secs` ago.
///
/// Defense-in-depth alongside the sender-side `mark_invite_responded`
/// single-use enforcement: even if an attacker harvests a link and
/// presents it to multiple receivers, the recency window caps how long
/// the harvest remains useful. Vulnerable-user safety stance: leaked
/// links shouldn't grant indefinite reach. Default policy in
/// `add_friend_from_invite` is 7 days.
///
/// `now_ms` is supplied by the caller (rather than read from the
/// system clock) so this stays as a pure protocol helper. An invite
/// with `issued_at == 0` is rejected as a pre-recency-field blob —
/// the sender must regenerate. No legacy fallback.
pub fn check_invite_recency(
    blob: &InviteBlob,
    now_ms: u64,
    max_age_secs: u64,
) -> Result<(), String> {
    if blob.issued_at == 0 {
        return Err(
            "invite is missing issued_at — please ask the sender to regenerate the invite link"
                .to_string(),
        );
    }
    let age_ms = now_ms.saturating_sub(blob.issued_at);
    let max_age_ms = max_age_secs.saturating_mul(1_000);
    if age_ms > max_age_ms {
        let age_days = age_ms / (24 * 3600 * 1000);
        let max_age_days = max_age_secs / (24 * 3600);
        return Err(format!(
            "invite expired ({age_days}d old, max {max_age_days}d) — please ask the sender to generate a new invite"
        ));
    }
    Ok(())
}

/// Encode an invite blob as a `rekindle://` URL.
pub fn encode_invite_url(blob: &InviteBlob) -> String {
    let json = serde_json::to_vec(blob).unwrap_or_default();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&json);
    format!("rekindle://{encoded}")
}

/// Decode an invite blob from a `rekindle://` URL or raw base64 string.
pub fn decode_invite_url(url: &str) -> Result<InviteBlob, String> {
    let data = url.strip_prefix("rekindle://").unwrap_or(url);
    let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|e| format!("invalid base64: {e}"))?;
    let blob: InviteBlob =
        serde_json::from_slice(&json_bytes).map_err(|e| format!("invalid invite JSON: {e}"))?;
    Ok(blob)
}

#[cfg(test)]
#[path = "invite_invite_blob_tests.rs"]
mod invite_blob_tests;
