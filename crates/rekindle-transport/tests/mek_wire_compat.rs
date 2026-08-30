//! The MEK wire format, as three crates actually implement it.
//!
//! `[generation LE(8) || key(32)]` is written and read by three separate
//! types: `rekindle_secrets::keys::MediaEncryptionKey`,
//! `rekindle_crypto::group::media_key::MediaEncryptionKey`, and
//! `rekindle_transport::crypto::mek::Mek`. This crate is the only one
//! that can see all three, so the compatibility matrix is pinned here.
//!
//! They are NOT interchangeable, and the difference is deliberate:
//!
//!   * secrets and transport implement the base 40-byte form.
//!   * crypto extends it with an optional 65-byte provenance suffix —
//!     `flag(1)=1 || rotator_pseudonym(32) || election_rank(32)` — for a
//!     105-byte total, documented as backward compatible.
//!
//! A reader of the base form truncates a 105-byte blob to its first 40,
//! silently discarding provenance. That is safe for transport, which
//! carries the rotator identity in the transfer envelope
//! (`rotator_pseudonym_hex`) rather than inside the key blob, but it is
//! exactly the kind of asymmetry that becomes a bug the moment someone
//! "unifies" these three by deleting two of them. These tests state
//! which direction is lossy so that a future merge has to confront it.

use rekindle_crypto::group::media_key::MediaEncryptionKey as CryptoMek;
use rekindle_secrets::keys::MediaEncryptionKey as SecretsMek;
use rekindle_transport::crypto::mek::Mek as TransportMek;

const KEY: [u8; 32] = [0xAB; 32];
const GENERATION: u64 = 7;

/// The base form is byte-identical across all three implementations.
#[test]
fn base_form_is_identical_in_all_three() {
    let secrets = SecretsMek::from_bytes(KEY, GENERATION).to_wire_bytes();
    let crypto = CryptoMek::from_bytes(KEY, GENERATION).to_wire_bytes();
    let transport = TransportMek::from_bytes(KEY, GENERATION).to_wire_bytes();

    assert_eq!(secrets.len(), 40, "base form is 40 bytes");
    assert_eq!(secrets, crypto, "secrets and crypto must agree");
    assert_eq!(secrets, transport, "secrets and transport must agree");

    // Pin the layout itself: generation little-endian, then the key.
    assert_eq!(&secrets[..8], &GENERATION.to_le_bytes());
    assert_eq!(&secrets[8..40], &KEY);
}

/// Every implementation reads a blob any other wrote, in the base form.
#[test]
fn base_form_round_trips_across_implementations() {
    let written = CryptoMek::from_bytes(KEY, GENERATION).to_wire_bytes();

    let by_secrets = SecretsMek::from_wire_bytes(&written).expect("secrets reads crypto's bytes");
    assert_eq!(by_secrets.as_bytes(), &KEY);
    assert_eq!(by_secrets.generation(), GENERATION);

    let by_transport =
        TransportMek::from_wire_bytes(&written).expect("transport reads crypto's bytes");
    assert_eq!(by_transport.as_bytes(), &KEY);
    assert_eq!(by_transport.generation(), GENERATION);
}

/// The extended form exists only in `rekindle-crypto`, and its first 40
/// bytes are exactly the base form — which is what makes it readable by
/// the other two at all.
#[test]
fn extended_form_is_a_superset_of_the_base_form() {
    let base = CryptoMek::from_bytes(KEY, GENERATION).to_wire_bytes();
    let extended = CryptoMek::from_bytes(KEY, GENERATION)
        .with_provenance([0x11; 32], [0x22; 32])
        .to_wire_bytes();

    assert_eq!(extended.len(), 105, "provenance adds flag(1) + 32 + 32");
    assert_eq!(
        &extended[..40],
        &base[..],
        "the base form must be the prefix"
    );
    assert_eq!(extended[40], 1, "provenance-present flag");
}

/// Documents the lossy direction: the base-form readers accept a
/// provenance blob and silently drop the suffix. Safe today because
/// transport tracks the rotator in the transfer envelope instead — but a
/// merge that made either of them canonical would lose it outright.
#[test]
fn base_form_readers_truncate_provenance_rather_than_failing() {
    let extended = CryptoMek::from_bytes(KEY, GENERATION)
        .with_provenance([0x11; 32], [0x22; 32])
        .to_wire_bytes();

    let by_transport =
        TransportMek::from_wire_bytes(&extended).expect("must accept, not reject, the long form");
    assert_eq!(by_transport.as_bytes(), &KEY);
    assert_eq!(by_transport.generation(), GENERATION);

    let by_secrets = SecretsMek::from_wire_bytes(&extended).expect("must accept the long form");
    assert_eq!(by_secrets.as_bytes(), &KEY);

    // Round-tripping through a base-form reader loses the provenance.
    assert_eq!(
        by_transport.to_wire_bytes().len(),
        40,
        "re-encoding through a base-form type drops the suffix"
    );

    // Only crypto recovers it.
    let by_crypto = CryptoMek::from_wire_bytes(&extended).expect("crypto reads its own form");
    assert_eq!(by_crypto.rotator_pseudonym(), Some([0x11; 32]));
    assert_eq!(by_crypto.election_rank(), Some([0x22; 32]));
}

/// A 40-byte blob loads into the extended reader with no provenance —
/// the backward-compatibility claim in `to_wire_bytes`' doc comment.
#[test]
fn extended_reader_accepts_the_base_form() {
    let base = TransportMek::from_bytes(KEY, GENERATION).to_wire_bytes();
    let by_crypto = CryptoMek::from_wire_bytes(&base).expect("crypto reads the base form");
    assert_eq!(by_crypto.as_bytes(), &KEY);
    assert_eq!(by_crypto.generation(), GENERATION);
    assert_eq!(by_crypto.rotator_pseudonym(), None);
    assert_eq!(by_crypto.election_rank(), None);
}

/// Short input is rejected by all three rather than partially parsed.
#[test]
fn all_three_reject_short_input() {
    let short = vec![0u8; 39];
    assert!(SecretsMek::from_wire_bytes(&short).is_none());
    assert!(CryptoMek::from_wire_bytes(&short).is_none());
    assert!(TransportMek::from_wire_bytes(&short).is_none());
}
