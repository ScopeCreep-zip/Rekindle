use super::*;

#[test]
fn mek_encrypt_decrypt_roundtrip() {
    let mek = MediaEncryptionKey::generate(1);
    let plaintext = b"hello from a community channel";
    let encrypted = mek.encrypt(plaintext).unwrap();
    let decrypted = mek.decrypt(&encrypted).unwrap();
    assert_eq!(plaintext.as_slice(), &decrypted);
}

#[test]
fn mek_wire_bytes_roundtrip() {
    let mek = MediaEncryptionKey::generate(42);
    let wire = mek.to_wire_bytes();
    assert_eq!(wire.len(), 40);
    let restored = MediaEncryptionKey::from_wire_bytes(&wire).unwrap();
    assert_eq!(restored.generation(), 42);
    assert_eq!(restored.as_bytes(), mek.as_bytes());
}

#[test]
fn mek_wire_bytes_too_short() {
    assert!(MediaEncryptionKey::from_wire_bytes(&[0u8; 39]).is_none());
    assert!(MediaEncryptionKey::from_wire_bytes(&[]).is_none());
}

#[test]
fn different_keys_fail_decrypt() {
    let mek1 = MediaEncryptionKey::generate(1);
    let mek2 = MediaEncryptionKey::generate(2);
    let encrypted = mek1.encrypt(b"secret").unwrap();
    assert!(mek2.decrypt(&encrypted).is_err());
}

/// AAD binds a ciphertext to its channel and stream position.
/// Moved here with `encrypt_with_aad`; it had no coverage in its
/// previous home either, which is not a reason to leave a Tier 2
/// crypto path untested.
#[test]
fn aad_roundtrip() {
    let mek = MediaEncryptionKey::generate(1);
    let aad = ChannelAad {
        anchor: "VLD0:anchor",
        channel_id: "chan-a",
        author: &[9; 32],
        stream_seq: 99,
    };
    let ct = mek
        .encrypt_with_aad(b"hello", &aad.to_bytes())
        .expect("encrypt");
    let pt = mek.decrypt_with_aad(&ct, &aad.to_bytes()).expect("decrypt");
    assert_eq!(pt, b"hello");
}

/// The point of the AAD: a ciphertext lifted into a different
/// channel, subkey, or stream position must not decrypt.
#[test]
fn aad_mismatch_fails_decrypt() {
    let mek = MediaEncryptionKey::generate(1);
    let aad = ChannelAad {
        anchor: "VLD0:anchor",
        channel_id: "chan-a",
        author: &[9; 32],
        stream_seq: 99,
    };
    let ct = mek
        .encrypt_with_aad(b"hello", &aad.to_bytes())
        .expect("encrypt");

    for wrong in [
        ChannelAad {
            channel_id: "chan-b",
            ..aad
        },
        ChannelAad {
            author: &[8; 32],
            ..aad
        },
        ChannelAad {
            stream_seq: 100,
            ..aad
        },
    ] {
        assert!(
            mek.decrypt_with_aad(&ct, &wrong.to_bytes()).is_err(),
            "a ciphertext must not decrypt under a different AAD"
        );
    }
}

/// An AAD-less ciphertext and an AAD-bound one are not
/// interchangeable in either direction.
#[test]
fn aad_and_plain_paths_do_not_cross() {
    let mek = MediaEncryptionKey::generate(1);
    let aad = ChannelAad {
        anchor: "VLD0:anchor",
        channel_id: "chan-a",
        author: &[9; 32],
        stream_seq: 99,
    };
    let plain = mek.encrypt(b"hi").expect("encrypt");
    assert!(mek.decrypt_with_aad(&plain, &aad.to_bytes()).is_err());

    let bound = mek
        .encrypt_with_aad(b"hi", &aad.to_bytes())
        .expect("encrypt");
    assert!(mek.decrypt(&bound).is_err());
}

/// A reader that takes exactly the first 40 bytes of a tagged key
/// gets a valid base-form key. This is what makes the provenance
/// suffix append-only rather than a breaking change.
#[test]
fn tagged_key_decodes_from_its_40_byte_prefix() {
    let tagged = MediaEncryptionKey::from_bytes([9u8; 32], 4)
        .with_provenance([1u8; 32], [2u8; 32])
        .to_wire_bytes();
    assert_eq!(tagged.len(), 105);

    let prefix = MediaEncryptionKey::from_wire_bytes(&tagged[..40]).expect("decodes");
    assert_eq!(prefix.as_bytes(), &[9u8; 32]);
    assert_eq!(prefix.generation(), 4);
    assert_eq!(prefix.election_rank(), None);
}

#[test]
fn slot_seed_generates_random() {
    let s1 = SlotSeed::generate_with(&rekindle_utils::random::OsRandom);
    let s2 = SlotSeed::generate_with(&rekindle_utils::random::OsRandom);
    assert_ne!(s1.0, s2.0);
}
