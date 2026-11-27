use super::*;
use crate::derive::derive_community_pseudonym;
use crate::keys::MediaEncryptionKey;

#[test]
fn wrap_unwrap_roundtrip() {
    let sender_key = derive_community_pseudonym(&[1u8; 32], "test");
    let recipient_key = derive_community_pseudonym(&[2u8; 32], "test");
    let mek = MediaEncryptionKey::generate(42);
    let wire = mek.to_wire_bytes();

    let wrapped = wrap_mek(
        &sender_key,
        &recipient_key.verifying_key().to_bytes(),
        &wire,
    )
    .unwrap();
    assert_eq!(wrapped.len(), 68);

    let unwrapped = unwrap_mek(
        &recipient_key,
        &sender_key.verifying_key().to_bytes(),
        &wrapped,
    )
    .unwrap();
    assert_eq!(unwrapped, wire);

    let restored = MediaEncryptionKey::from_wire_bytes(&unwrapped).unwrap();
    assert_eq!(restored.generation(), 42);
    assert_eq!(restored.as_bytes(), mek.as_bytes());
}

#[test]
fn wrong_recipient_fails() {
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    let wrong = derive_community_pseudonym(&[3u8; 32], "c");

    let mek = MediaEncryptionKey::generate(1);
    let wrapped = wrap_mek(
        &sender,
        &recipient.verifying_key().to_bytes(),
        &mek.to_wire_bytes(),
    )
    .unwrap();

    assert!(unwrap_mek(&wrong, &sender.verifying_key().to_bytes(), &wrapped,).is_err());
}

#[test]
fn wrong_sender_key_fails() {
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let fake_sender = derive_community_pseudonym(&[99u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");

    let mek = MediaEncryptionKey::generate(1);
    let wrapped = wrap_mek(
        &sender,
        &recipient.verifying_key().to_bytes(),
        &mek.to_wire_bytes(),
    )
    .unwrap();

    assert!(unwrap_mek(
        &recipient,
        &fake_sender.verifying_key().to_bytes(),
        &wrapped,
    )
    .is_err());
}

#[test]
fn wrapped_too_short() {
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    assert!(unwrap_mek(&recipient, &sender.verifying_key().to_bytes(), &[0u8; 11],).is_err());
}

#[test]
fn hpke_wrap_open_roundtrip() {
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    let mek = MediaEncryptionKey::generate(7);
    let wire = mek.to_wire_bytes();

    let wrapped = hpke_wrap_mek(&sender, &recipient.verifying_key().to_bytes(), &wire).unwrap();
    assert_eq!(wrapped[0], HPKE_MEK_VERSION);
    // 1 version + 32 enc + 40 plaintext + 16 tag = 89
    assert_eq!(wrapped.len(), 89);

    let opened = hpke_open_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).unwrap();
    assert_eq!(opened, wire);
}

/// A v2 blob sealed before the wrap moved onto `crate::hpke` (sender
/// and recipient as below, plaintext `0..40`) still opens: the move
/// left the wire bytes and the key schedule unchanged.
#[test]
fn hpke_blob_from_before_the_wrapper_still_opens() {
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    let wrapped = hex::decode(
        "025a004e4c8adf32f6f8c74f736afdfeea606457d2465b4c9f9c6961daf5f619\
             197dc7f7eb346b2ba5763be8cb430b194c925c814c7b4e849d802a5e59ec07e0\
             162150d264afa53ad060726c5f696719d48aab37dbd1ec6ff4",
    )
    .unwrap();
    let opened = hpke_open_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).unwrap();
    assert_eq!(opened, (0u8..40).collect::<Vec<_>>());
}

#[test]
fn hpke_blob_opens_through_dual_read_unwrap() {
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    let mek = MediaEncryptionKey::generate(9);
    let wire = mek.to_wire_bytes();

    let wrapped = hpke_wrap_mek(&sender, &recipient.verifying_key().to_bytes(), &wire).unwrap();
    let opened = unwrap_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).unwrap();
    assert_eq!(opened, wire);
}

#[test]
fn hpke_auth_mode_rejects_wrong_sender() {
    // Auth mode preserves v1's sender authentication: opening with
    // the wrong claimed sender must fail.
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let fake_sender = derive_community_pseudonym(&[9u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    let mek = MediaEncryptionKey::generate(1);

    let wrapped = hpke_wrap_mek(
        &sender,
        &recipient.verifying_key().to_bytes(),
        &mek.to_wire_bytes(),
    )
    .unwrap();
    assert!(hpke_open_mek(
        &recipient,
        &fake_sender.verifying_key().to_bytes(),
        &wrapped
    )
    .is_err());
}

#[test]
fn hpke_tampered_ciphertext_rejected() {
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    let mek = MediaEncryptionKey::generate(1);

    let mut wrapped = hpke_wrap_mek(
        &sender,
        &recipient.verifying_key().to_bytes(),
        &mek.to_wire_bytes(),
    )
    .unwrap();
    let last = wrapped.len() - 1;
    wrapped[last] ^= 0xFF;
    assert!(hpke_open_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).is_err());
}

#[test]
fn v1_blob_with_version_colliding_nonce_still_opens() {
    // A legacy v1 blob whose random nonce happens to start with the
    // v2 version byte must fall through the failed HPKE attempt and
    // decrypt via the v1 path. Expected ~1 collision per 256 wraps.
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    let mek = MediaEncryptionKey::generate(3);
    let wire = mek.to_wire_bytes();

    for _ in 0..10_000 {
        let wrapped = wrap_mek(&sender, &recipient.verifying_key().to_bytes(), &wire).unwrap();
        if wrapped[0] == HPKE_MEK_VERSION {
            let opened =
                unwrap_mek(&recipient, &sender.verifying_key().to_bytes(), &wrapped).unwrap();
            assert_eq!(opened, wire);
            return;
        }
    }
    panic!("no 0x02-leading v1 nonce in 10k wraps — statistically broken RNG");
}

#[test]
fn cross_community_fails() {
    let sender_a = derive_community_pseudonym(&[1u8; 32], "community_a");
    let recipient_a = derive_community_pseudonym(&[2u8; 32], "community_a");
    let recipient_b = derive_community_pseudonym(&[2u8; 32], "community_b");
    let sender_b = derive_community_pseudonym(&[1u8; 32], "community_b");

    let mek = MediaEncryptionKey::generate(1);
    let wrapped = wrap_mek(
        &sender_a,
        &recipient_a.verifying_key().to_bytes(),
        &mek.to_wire_bytes(),
    )
    .unwrap();

    assert!(unwrap_mek(&recipient_b, &sender_b.verifying_key().to_bytes(), &wrapped,).is_err());
}
