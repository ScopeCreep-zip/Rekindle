use super::*;
use crate::group::media_key::MediaEncryptionKey;
use crate::group::pseudonym::derive_community_pseudonym;

#[test]
fn wrap_unwrap_roundtrip() {
    let coordinator_secret = [1u8; 32];
    let member_secret = [2u8; 32];
    let community_id = "test_community";

    let coordinator_key = derive_community_pseudonym(&coordinator_secret, community_id);
    let member_key = derive_community_pseudonym(&member_secret, community_id);

    let mek = MediaEncryptionKey::generate(42);
    let mek_wire = mek.to_wire_bytes();

    // Coordinator wraps MEK for member
    let wrapped = wrap_mek(
        &coordinator_key,
        &member_key.verifying_key().to_bytes(),
        &mek_wire,
    )
    .unwrap();

    // Expected: 12 nonce + 40 plaintext + 16 tag = 68 bytes
    assert_eq!(wrapped.len(), 68);

    // Member unwraps MEK
    let unwrapped = unwrap_mek(
        &member_key,
        &coordinator_key.verifying_key().to_bytes(),
        &wrapped,
    )
    .unwrap();

    assert_eq!(unwrapped, mek_wire);

    // Verify the unwrapped MEK has correct generation and key material
    let restored = MediaEncryptionKey::from_wire_bytes(&unwrapped).unwrap();
    assert_eq!(restored.generation(), 42);
    assert_eq!(restored.as_bytes(), mek.as_bytes());
}

#[test]
fn wrong_recipient_cannot_unwrap() {
    let coordinator_secret = [1u8; 32];
    let member_secret = [2u8; 32];
    let wrong_member_secret = [3u8; 32];
    let community_id = "test_community";

    let coordinator_key = derive_community_pseudonym(&coordinator_secret, community_id);
    let member_key = derive_community_pseudonym(&member_secret, community_id);
    let wrong_member_key = derive_community_pseudonym(&wrong_member_secret, community_id);

    let mek = MediaEncryptionKey::generate(1);
    let wrapped = wrap_mek(
        &coordinator_key,
        &member_key.verifying_key().to_bytes(),
        &mek.to_wire_bytes(),
    )
    .unwrap();

    // Wrong member tries to unwrap — should fail
    let result = unwrap_mek(
        &wrong_member_key,
        &coordinator_key.verifying_key().to_bytes(),
        &wrapped,
    );
    assert!(result.is_err());
}

#[test]
fn wrong_sender_key_cannot_unwrap() {
    let coordinator_secret = [1u8; 32];
    let fake_coordinator_secret = [99u8; 32];
    let member_secret = [2u8; 32];
    let community_id = "test_community";

    let coordinator_key = derive_community_pseudonym(&coordinator_secret, community_id);
    let fake_coordinator_key = derive_community_pseudonym(&fake_coordinator_secret, community_id);
    let member_key = derive_community_pseudonym(&member_secret, community_id);

    let mek = MediaEncryptionKey::generate(1);
    let wrapped = wrap_mek(
        &coordinator_key,
        &member_key.verifying_key().to_bytes(),
        &mek.to_wire_bytes(),
    )
    .unwrap();

    // Member tries to unwrap with wrong sender public key — should fail
    let result = unwrap_mek(
        &member_key,
        &fake_coordinator_key.verifying_key().to_bytes(),
        &wrapped,
    );
    assert!(result.is_err());
}

#[test]
fn wrapped_mek_too_short() {
    let coordinator_secret = [1u8; 32];
    let member_secret = [2u8; 32];
    let coordinator_key = derive_community_pseudonym(&coordinator_secret, "c");
    let member_key = derive_community_pseudonym(&member_secret, "c");

    let result = unwrap_mek(
        &member_key,
        &coordinator_key.verifying_key().to_bytes(),
        &[0u8; 11], // too short
    );
    assert!(result.is_err());
}

#[test]
fn different_communities_different_wrapping_keys() {
    let coordinator_secret = [1u8; 32];
    let member_secret = [2u8; 32];

    let coord_a = derive_community_pseudonym(&coordinator_secret, "community_a");
    let member_a = derive_community_pseudonym(&member_secret, "community_a");

    let coord_b = derive_community_pseudonym(&coordinator_secret, "community_b");
    let member_b = derive_community_pseudonym(&member_secret, "community_b");

    let mek = MediaEncryptionKey::generate(1);
    let wrapped_a = wrap_mek(
        &coord_a,
        &member_a.verifying_key().to_bytes(),
        &mek.to_wire_bytes(),
    )
    .unwrap();

    // Cannot unwrap community_a's MEK with community_b's keys
    let result = unwrap_mek(&member_b, &coord_b.verifying_key().to_bytes(), &wrapped_a);
    assert!(result.is_err());
}

#[test]
fn output_size_is_68_bytes() {
    let sender = derive_community_pseudonym(&[1u8; 32], "c");
    let recipient = derive_community_pseudonym(&[2u8; 32], "c");
    let mek = MediaEncryptionKey::generate(1);
    let wire = mek.to_wire_bytes();
    assert_eq!(wire.len(), 40);

    let wrapped = wrap_mek(&sender, &recipient.verifying_key().to_bytes(), &wire).unwrap();
    // 12 nonce + 40 plaintext + 16 GCM tag = 68
    assert_eq!(wrapped.len(), 68);
}
