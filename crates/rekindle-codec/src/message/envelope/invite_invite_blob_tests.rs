use super::*;

fn inbox() -> InboxCapability {
    InboxCapability {
        record_key: "VLD0:inbox".into(),
        writer: "VLD0:pub:secret".into(),
    }
}

/// The longest inbox: a record key with its encryption half and a
/// writer's keypair text, each half 43 base64url characters.
fn worst_inbox() -> InboxCapability {
    let half = "A".repeat(43);
    InboxCapability {
        record_key: format!("VLD0:{half}:{half}"),
        writer: format!("VLD0:{half}:{half}"),
    }
}

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
        &inbox(),
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
    let mut blob = create_invite_blob(&secret, &pk, "Alice", "mb", "pr", &inbox(), None, 7);
    let signed = signable_bytes(&pk, "Alice", "mb", "pr", &inbox(), None, 7);
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
        "inbox": { "record_key": "VLD0:k", "writer": "VLD0:p:s" },
        "invite_id": Some("inv-old"),
    });
    let signable_bytes = serde_json::to_vec(&pre_b11_signable).unwrap();
    let signature = signing.sign(&signable_bytes);
    let blob = InviteBlob {
        public_key: pk,
        display_name: "Alice".to_string(),
        mailbox_dht_key: "VLD0:mailbox".to_string(),
        profile_dht_key: "VLD0:profile".to_string(),
        inbox: inbox(),
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
        &inbox(),
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
        &inbox(),
        None,
        1_715_000_000_000,
    );
    blob.issued_at = 0;
    let err = check_invite_recency(&blob, 1_715_000_000_000, 7 * 24 * 3600)
        .expect_err("issued_at=0 must be rejected");
    assert!(err.contains("missing issued_at"), "got: {err}",);
}

/// The worst-case friend invite fits the OS deep-link cap: a 64-char
/// display name of 4-byte characters, record keys with their
/// encryption halves, the inbox writer's keypair text, an invite id
/// and the longest `issued_at` (plans C7.25, E2.5).
#[test]
fn the_worst_case_invite_url_fits_the_deep_link_cap() {
    use rekindle_types::invite::{DeepLink, MAX_FRIEND_INVITE_URL_LEN};
    let secret = make_secret();
    let pk = pubkey_hex(&secret);
    let half = "A".repeat(43);
    let key = format!("VLD0:{half}:{half}");
    let blob = create_invite_blob(
        &secret,
        &pk,
        &"😀".repeat(rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN),
        &key,
        &key,
        &worst_inbox(),
        Some("6f1c2d3e-4a5b-4c6d-8e7f-9a0b1c2d3e4f"),
        u64::MAX,
    );
    let url = encode_invite_url(&blob);
    assert!(url.len() <= MAX_FRIEND_INVITE_URL_LEN, "{} B", url.len());
    assert!(matches!(
        DeepLink::parse(&url),
        Ok(DeepLink::FriendInvite(_))
    ));
    let decoded = decode_invite_url(&url).unwrap();
    verify_invite_blob(&decoded).expect("the decoded invite verifies");
    assert_eq!(decoded.inbox, worst_inbox());
}
