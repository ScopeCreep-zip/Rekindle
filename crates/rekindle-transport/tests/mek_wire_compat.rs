//! The MEK wire format, now that one type implements it.
//!
//! This file used to pin a three-way compatibility matrix:
//! `rekindle_secrets::keys::MediaEncryptionKey`,
//! `rekindle_crypto::group::media_key::MediaEncryptionKey` and
//! `rekindle_transport::crypto::mek::Mek` each had their own copy of
//! `[generation LE(8) || key(32)]`, and only crypto's carried the
//! optional 65-byte provenance suffix. It closed with:
//!
//! > it is exactly the kind of asymmetry that becomes a bug the moment
//! > someone "unifies" these three by deleting two of them.
//!
//! It became a bug before anyone unified anything. The daemon's
//! `MekCacheAdapter` converted into the base form on every insert,
//! dropping the `election_rank` that
//! `convergence::incoming_wins_same_generation` needs, so the ranks had
//! to be carried in a side map — a workaround for a truncation that
//! existed only because there were three types.
//!
//! There is now one, `rekindle_secrets::keys::MediaEncryptionKey`, which
//! the other two re-export. So the matrix is gone and what remains is
//! the thing that outlives any refactor: **peers on the network parse
//! these bytes**, and the layout must not drift. Byte offsets are
//! asserted literally rather than by round-trip, because a round-trip
//! test passes happily while both ends move together.

use rekindle_crypto::group::media_key::MediaEncryptionKey as CryptoMek;
use rekindle_secrets::keys::MediaEncryptionKey as SecretsMek;
use rekindle_transport::crypto::mek::Mek as TransportMek;

const KEY: [u8; 32] = [0xAB; 32];
const GENERATION: u64 = 7;
const ROTATOR: [u8; 32] = [0x11; 32];
const RANK: [u8; 32] = [0x22; 32];

/// The merge held: all three paths name one type. A future re-fork
/// fails here rather than silently reintroducing the divergence this
/// file used to document.
#[test]
fn all_three_paths_are_the_same_type() {
    // Only compiles if the three aliases resolve identically.
    let a: SecretsMek = SecretsMek::from_bytes(KEY, GENERATION);
    let b: CryptoMek = a.clone();
    let c: TransportMek = b.clone();
    assert_eq!(c.as_bytes(), &KEY);
    assert_eq!(c.generation(), GENERATION);
}

/// Base layout, asserted by offset: `[generation LE(8) || key(32)]`.
#[test]
fn base_form_layout_is_exact() {
    let wire = SecretsMek::from_bytes(KEY, GENERATION).to_wire_bytes();

    assert_eq!(wire.len(), 40, "base form is 40 bytes");
    assert_eq!(
        &wire[..8],
        &GENERATION.to_le_bytes(),
        "generation is little-endian in bytes 0..8"
    );
    assert_eq!(&wire[8..40], &KEY, "key occupies bytes 8..40");
}

/// Extended layout: the base 40 unchanged, then
/// `[flag(1)=1 || rotator(32) || rank(32)]`.
#[test]
fn extended_form_layout_is_exact() {
    let base = SecretsMek::from_bytes(KEY, GENERATION).to_wire_bytes();
    let wire = SecretsMek::from_bytes(KEY, GENERATION)
        .with_provenance(ROTATOR, RANK)
        .to_wire_bytes();

    assert_eq!(wire.len(), 105, "extended form is 105 bytes");
    assert_eq!(
        &wire[..40],
        &base[..],
        "the first 40 bytes are the base form, unchanged"
    );
    assert_eq!(wire[40], 1, "byte 40 is the provenance-present flag");
    assert_eq!(&wire[41..73], &ROTATOR, "rotator occupies bytes 41..73");
    assert_eq!(
        &wire[73..105],
        &RANK,
        "election rank occupies bytes 73..105"
    );
}

/// Provenance survives a round trip. This is what the side map used to
/// have to reproduce by hand.
#[test]
fn provenance_round_trips() {
    let wire = SecretsMek::from_bytes(KEY, GENERATION)
        .with_provenance(ROTATOR, RANK)
        .to_wire_bytes();
    let back = SecretsMek::from_wire_bytes(&wire).expect("decode");

    assert_eq!(back.as_bytes(), &KEY);
    assert_eq!(back.generation(), GENERATION);
    assert_eq!(back.rotator_pseudonym(), Some(ROTATOR));
    assert_eq!(back.election_rank(), Some(RANK));
}

/// A 40-byte blob from a peer that predates provenance still decodes,
/// with `None` rather than an error. The suffix is append-only, which is
/// what let the extended form ship without a flag day.
#[test]
fn a_base_form_blob_decodes_with_no_provenance() {
    let wire = SecretsMek::from_bytes(KEY, GENERATION).to_wire_bytes();
    let back = SecretsMek::from_wire_bytes(&wire).expect("decode");

    assert_eq!(back.as_bytes(), &KEY);
    assert_eq!(back.generation(), GENERATION);
    assert_eq!(back.rotator_pseudonym(), None);
    assert_eq!(back.election_rank(), None);
}

/// A truncated provenance suffix must not be half-read: the flag says
/// provenance is present, but the bytes are not all there, so the key
/// decodes as base-form rather than with a partially-copied rank.
#[test]
fn a_truncated_suffix_falls_back_to_base_form() {
    let full = SecretsMek::from_bytes(KEY, GENERATION)
        .with_provenance(ROTATOR, RANK)
        .to_wire_bytes();

    for cut in [41, 60, 104] {
        let back = SecretsMek::from_wire_bytes(&full[..cut]).expect("base form still decodes");
        assert_eq!(back.as_bytes(), &KEY, "key survives at cut {cut}");
        assert_eq!(
            back.election_rank(),
            None,
            "a partial suffix must yield no rank at cut {cut}, never a truncated one"
        );
    }
}

/// Anything shorter than the base form is not a key.
#[test]
fn short_input_is_rejected() {
    for len in [0usize, 8, 39] {
        let bytes = vec![0u8; len];
        assert!(
            SecretsMek::from_wire_bytes(&bytes).is_none(),
            "{len} bytes must not decode"
        );
    }
}
