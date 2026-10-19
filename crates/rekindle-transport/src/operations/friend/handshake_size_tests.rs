//! Every friend-handshake envelope fits Veilid's message cap (plan C7.25).
//!
//! FriendAccept was ~37 KB and FriendRequest ~33 KB against Veilid's
//! 32,768-byte `app_message` limit while the prekey bundle travelled as
//! JSON inside the JSON payload, so every invite failed. These build each
//! handshake payload at its worst case, from a real handout bundle,
//! serialize it as `seal` does for a `Sealing::Plain` payload, sign it
//! with the protocol's envelope builder and wire it as every 1:1 send
//! does (`operations::dm::wire`, the `'D'` tag included).

use ed25519_dalek::SigningKey;
use rekindle_codec::message::envelope::{MessagePayload, Sealing};
use rekindle_crypto::signal::{
    MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore, SignalSessionManager,
};
use rekindle_protocol::messaging::sender::build_envelope;
use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

/// 3.5× the largest 1-hop route blob measured (1,178 B), room for 3-hop
/// inbound routes.
const ROUTE_BLOB_LEN: usize = 4_096;
/// ML-KEM-768 ciphertext.
const ML_KEM_CIPHERTEXT_LEN: usize = 1_088;

fn handout_bundle_wire() -> Vec<u8> {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let manager = SignalSessionManager::new(
        Box::new(MemoryIdentityStore::new(
            signing.to_bytes().to_vec(),
            signing.verifying_key().as_bytes().to_vec(),
            1,
        )),
        Box::new(MemoryPreKeyStore::new()),
        Box::new(MemorySessionStore::new()),
    );
    manager.handout_bundle().unwrap().to_wire()
}

/// A display name and message at their bounds, every character 4 bytes.
fn widest(chars: usize) -> String {
    "😀".repeat(chars)
}

fn dht_key() -> String {
    format!("VLD0:{}", "A".repeat(43))
}

fn encoded_len(payload: &MessagePayload) -> usize {
    assert_eq!(
        payload.sealing(),
        Sealing::Plain,
        "handshake payloads travel unsealed"
    );
    let sealed = serde_json::to_vec(payload).unwrap();
    let signing = SigningKey::from_bytes(&[9; 32]);
    let envelope = build_envelope(&signing, &[3; 32], u64::MAX, vec![0; 16], sealed);
    crate::operations::dm::wire(&envelope)
        .unwrap_or_else(|e| panic!("{payload:?} does not fit: {e}"))
        .len()
}

#[test]
fn friend_request_fits() {
    let len = encoded_len(&MessagePayload::FriendRequest {
        display_name: widest(rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN),
        message: widest(rekindle_types::key_format::MAX_MESSAGE_LEN),
        prekey_bundle: handout_bundle_wire(),
        profile_dht_key: dht_key(),
        route_blob: vec![0xAB; ROUTE_BLOB_LEN],
        mailbox_dht_key: dht_key(),
        invite_id: Some(uuid::Uuid::new_v4().to_string()),
    });
    assert!(len <= MAX_APP_MESSAGE_LEN, "FriendRequest {len} B");
}

#[test]
fn friend_accept_fits() {
    let len = encoded_len(&MessagePayload::FriendAccept {
        prekey_bundle: handout_bundle_wire(),
        profile_dht_key: dht_key(),
        route_blob: vec![0xAB; ROUTE_BLOB_LEN],
        mailbox_dht_key: dht_key(),
        ephemeral_key: vec![0xCD; 32],
        signed_prekey_id: u32::MAX,
        one_time_prekey_id: Some(u32::MAX),
        ml_kem_ciphertext: vec![0xEF; ML_KEM_CIPHERTEXT_LEN],
        used_ot_pqpk_id: Some(u32::MAX),
    });
    assert!(len <= MAX_APP_MESSAGE_LEN, "FriendAccept {len} B");
}

#[test]
fn session_reset_fits() {
    let request = encoded_len(&MessagePayload::SessionResetRequest {
        our_prekey_bundle: handout_bundle_wire(),
    });
    let accept = encoded_len(&MessagePayload::SessionResetAccept {
        ephemeral_key: vec![0xCD; 32],
        signed_prekey_id: u32::MAX,
        one_time_prekey_id: Some(u32::MAX),
        our_identity_key: vec![0x11; 32],
        ml_kem_ciphertext: vec![0xEF; ML_KEM_CIPHERTEXT_LEN],
        used_ot_pqpk_id: Some(u32::MAX),
    });
    assert!(
        request <= MAX_APP_MESSAGE_LEN,
        "SessionResetRequest {request} B"
    );
    assert!(
        accept <= MAX_APP_MESSAGE_LEN,
        "SessionResetAccept {accept} B"
    );
}

#[test]
fn an_oversize_envelope_is_refused_before_veilid() {
    let signing = SigningKey::from_bytes(&[9; 32]);
    let envelope = build_envelope(
        &signing,
        &[3; 32],
        0,
        vec![0; 16],
        vec![0x5A; MAX_APP_MESSAGE_LEN],
    );
    assert!(matches!(
        crate::operations::dm::wire(&envelope),
        Err(crate::operations::friend::FriendError::Invalid(_))
    ));
}
