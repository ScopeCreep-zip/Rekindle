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
    /// Sender's current route blob (for immediate contact, may be stale).
    #[serde(with = "rekindle_types::base64_bytes")]
    pub route_blob: Vec<u8>,
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
    route_blob: &[u8],
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
        route_blob,
        invite_id,
        issued_at_ms,
    ));

    InviteBlob {
        public_key: public_key.to_string(),
        display_name: display_name.to_string(),
        mailbox_dht_key: mailbox_dht_key.to_string(),
        profile_dht_key: profile_dht_key.to_string(),
        route_blob: route_blob.to_vec(),
        invite_id: invite_id.map(str::to_string),
        issued_at: issued_at_ms,
        signature: signature.to_bytes().to_vec(),
    }
}

/// The bytes an invite's signature covers: the registered domain
/// `FRIEND_INVITE_SIG`, then every field but the signature as JSON, with
/// the route blob base64 as the blob itself carries it. The domain keeps
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
    route_blob: &[u8],
    invite_id: Option<&str>,
    issued_at_ms: u64,
) -> Vec<u8> {
    let signable = serde_json::json!({
        "public_key": public_key,
        "display_name": display_name,
        "mailbox_dht_key": mailbox_dht_key,
        "profile_dht_key": profile_dht_key,
        "route_blob": base64::engine::general_purpose::STANDARD.encode(route_blob),
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
        &blob.route_blob,
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
mod invite_blob_tests {
    use super::*;

    fn make_secret() -> [u8; 32] {
        // Fixed test secret so the test is deterministic.
        let mut s = [0u8; 32];
        for (i, b) in s.iter_mut().enumerate() {
            // i is bounded 0..32 by the iterator, fits in u8.
            *b = u8::try_from(i).expect("i < 32").wrapping_add(1);
        }
        s
    }

    fn pubkey_hex(secret: &[u8; 32]) -> String {
        use rekindle_secrets::ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(secret);
        hex::encode(sk.verifying_key().to_bytes())
    }

    #[test]
    fn create_then_verify_round_trips_with_issued_at() {
        let secret = make_secret();
        let pk = pubkey_hex(&secret);
        let blob = create_invite_blob(
            &secret,
            &pk,
            "Alice",
            "VLD0:mailbox-key",
            "VLD0:profile-key",
            &[1u8, 2, 3],
            Some("inv-123"),
            1_715_000_000_000,
        );
        verify_invite_blob(&blob).expect("freshly-minted blob must verify on the same build");
        assert_eq!(blob.issued_at, 1_715_000_000_000);
    }

    /// The signature covers the registered domain then the JSON: the bare
    /// JSON signed alone (the pre-domain form) does not verify.
    #[test]
    fn the_signature_is_domain_separated() {
        use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
        let secret = make_secret();
        let pk = pubkey_hex(&secret);
        let mut blob = create_invite_blob(&secret, &pk, "Alice", "mb", "pr", &[], None, 7);
        let signed = signable_bytes(&pk, "Alice", "mb", "pr", &[], None, 7);
        let domain = rekindle_types::domains::FRIEND_INVITE_SIG.as_bytes();
        assert_eq!(&signed[..domain.len()], domain);
        let bare = SigningKey::from_bytes(&secret).sign(&signed[domain.len()..]);
        blob.signature = bare.to_bytes().to_vec();
        assert!(verify_invite_blob(&blob).is_err());
    }

    #[test]
    fn verify_rejects_pre_b11_blob_missing_issued_at() {
        // Simulate a pre-B11 sender by signing without issued_at, then
        // verifying with the new code path. The signature reconstruction
        // includes issued_at:0 (serde default for missing field), so the
        // canonical bytes differ from what the sender signed → mismatch.
        // This is the no-legacy-compat behavior: the user must regenerate
        // the invite on a current build.
        use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
        let secret = make_secret();
        let signing = SigningKey::from_bytes(&secret);
        let pk = pubkey_hex(&secret);
        let pre_b11_signable = serde_json::json!({
            "public_key": pk,
            "display_name": "Alice",
            "mailbox_dht_key": "VLD0:mailbox",
            "profile_dht_key": "VLD0:profile",
            "route_blob": "AQID",
            "invite_id": Some("inv-old"),
        });
        let signable_bytes = serde_json::to_vec(&pre_b11_signable).unwrap();
        let signature = signing.sign(&signable_bytes);
        let blob = InviteBlob {
            public_key: pk,
            display_name: "Alice".to_string(),
            mailbox_dht_key: "VLD0:mailbox".to_string(),
            profile_dht_key: "VLD0:profile".to_string(),
            route_blob: vec![1u8, 2, 3],
            invite_id: Some("inv-old".to_string()),
            issued_at: 0, // pre-B11 sender didn't include this field
            signature: signature.to_bytes().to_vec(),
        };
        let err = verify_invite_blob(&blob).expect_err("pre-B11 blob must be rejected");
        // The error message hints at the version mismatch instead of
        // showing the raw cryptographic error.
        assert!(err.contains("older app version"), "got: {err}",);
    }

    #[test]
    fn check_recency_within_window() {
        let secret = make_secret();
        let pk = pubkey_hex(&secret);
        let blob = create_invite_blob(
            &secret,
            &pk,
            "Alice",
            "mb",
            "pr",
            &[],
            None,
            1_715_000_000_000,
        );
        // 1 day after issuance
        let now = blob.issued_at + 24 * 3600 * 1000;
        check_invite_recency(&blob, now, 7 * 24 * 3600).expect("within window");
    }

    #[test]
    fn check_recency_rejects_zero_issued_at() {
        let secret = make_secret();
        let pk = pubkey_hex(&secret);
        let mut blob = create_invite_blob(
            &secret,
            &pk,
            "Alice",
            "mb",
            "pr",
            &[],
            None,
            1_715_000_000_000,
        );
        blob.issued_at = 0;
        let err = check_invite_recency(&blob, 1_715_000_000_000, 7 * 24 * 3600)
            .expect_err("issued_at=0 must be rejected");
        assert!(err.contains("missing issued_at"), "got: {err}",);
    }

    /// The worst-case friend invite fits the OS deep-link cap: a 64-char
    /// display name of 4-byte characters, a 4,096-byte route blob (3.5x
    /// the largest 1-hop blob measured) and an invite id (plan C7.25).
    #[test]
    fn the_worst_case_invite_url_fits_the_deep_link_cap() {
        use rekindle_types::invite::{DeepLink, MAX_FRIEND_INVITE_URL_LEN};
        let secret = make_secret();
        let pk = pubkey_hex(&secret);
        let key = format!("VLD0:{}", "A".repeat(43));
        let blob = create_invite_blob(
            &secret,
            &pk,
            &"😀".repeat(rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN),
            &key,
            &key,
            &[0xAB; 4_096],
            Some("6f1c2d3e-4a5b-4c6d-8e7f-9a0b1c2d3e4f"),
            1_715_000_000_000,
        );
        let url = encode_invite_url(&blob);
        assert!(url.len() <= MAX_FRIEND_INVITE_URL_LEN, "{} B", url.len());
        assert!(matches!(
            DeepLink::parse(&url),
            Ok(DeepLink::FriendInvite(_))
        ));
        let decoded = decode_invite_url(&url).unwrap();
        verify_invite_blob(&decoded).expect("the decoded invite verifies");
        assert_eq!(decoded.route_blob.len(), 4_096);
    }
}
