//! Safety numbers (plan E2.3): Signal's safety-number scheme, the 60
//! digits two people compare out of band, and its scannable (QR) form.
//!
//! Each side's fingerprint is an iterated SHA-512 over a two-byte version
//! (zero), the side's identity key and its stable identifier:
//! `H1 = SHA-512(0x0000 ‖ key ‖ id ‖ key)`, then
//! `Hi = SHA-512(Hi-1 ‖ key)` up to `H5200` ([`ITERATIONS`] hashes in
//! all). Its displayable form is the first 30 bytes of the result as six
//! 5-byte big-endian integers, each reduced `mod 100000` and written as
//! five zero-padded digits; the two sides' 30-digit strings are joined
//! lower first, so both peers show the same 60 digits. The scannable form
//! is a protobuf message (field 1 the varint version, fields 2 and 3 each
//! a nested message whose field 1 holds 32 bytes): the first 32 bytes of
//! our fingerprint, then of theirs. A scanner compares it crossed: the
//! code's first half against its own remote side and the second against
//! its own local side.
//!
//! Rekindle's inputs: the key is `EncodeEC` of the Ed25519 identity
//! (`0x06 ‖ key`, plan E2.2), and the stable identifier is the identity
//! key itself, because the identity is the address.

use sha2::{Digest, Sha512};

use crate::signal::pqxdh::encode::{ED25519_TYPE, ENCODED_EC_LEN};

/// Hash iterations per side (the scheme's 5200).
pub const ITERATIONS: u32 = 5200;

/// The scannable form's version: Signal's current one (2), whose stable
/// identifier is the account's service id rather than a phone number.
/// Rekindle's identifier is the identity key, which is the same kind of
/// stable, non-phone identifier.
pub const SCANNABLE_VERSION: u32 = 2;

/// Digits per side of the displayable number.
pub const DIGITS_PER_SIDE: usize = 30;

/// Bytes of each side carried in the scannable form.
const SCANNABLE_BYTES: usize = 32;

/// Length of the scannable form: `08 vv 12 22 0a 20 <32> 1a 22 0a 20 <32>`.
const SCANNABLE_LEN: usize = 2 + 2 * (4 + SCANNABLE_BYTES);

/// A scanned code that is not a safety number this build can compare.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FingerprintError {
    /// The code is another version of the format.
    #[error("safety number version {theirs}, ours is {ours}")]
    VersionMismatch { theirs: u32, ours: u32 },
    /// The bytes are not a scannable form of two 32-byte halves.
    #[error("not a scannable safety number")]
    Malformed,
}

/// The safety number of one pair of identities, from our side.
#[derive(Clone, PartialEq, Eq)]
pub struct SafetyNumber {
    local: [u8; 64],
    remote: [u8; 64],
}

impl std::fmt::Debug for SafetyNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SafetyNumber").field(&self.digits()).finish()
    }
}

impl SafetyNumber {
    /// The safety number between our identity `local` and the peer's
    /// `remote` (both Ed25519 identity keys).
    #[must_use]
    pub fn new(local: &[u8; 32], remote: &[u8; 32]) -> Self {
        Self {
            local: side(local),
            remote: side(remote),
        }
    }

    /// The 60 digits both peers see: each side's 30, lower first.
    #[must_use]
    pub fn digits(&self) -> String {
        joined(&self.local, &self.remote)
    }

    /// The QR payload: the version, our 32 bytes, then theirs.
    #[must_use]
    pub fn scannable(&self) -> Vec<u8> {
        scannable(SCANNABLE_VERSION, &self.local, &self.remote)
    }

    /// Whether a code scanned from the peer's screen shows this pair: its
    /// local half is our remote one and its remote half our local one.
    ///
    /// # Errors
    /// The code is malformed or another version.
    pub fn matches_scanned(&self, scanned: &[u8]) -> Result<bool, FingerprintError> {
        let (version, theirs_local, theirs_remote) = parse_scannable(scanned)?;
        if version != SCANNABLE_VERSION {
            return Err(FingerprintError::VersionMismatch {
                theirs: version,
                ours: SCANNABLE_VERSION,
            });
        }
        let same = equal(theirs_local, &self.remote[..SCANNABLE_BYTES])
            & equal(theirs_remote, &self.local[..SCANNABLE_BYTES]);
        Ok(same)
    }
}

/// One side's fingerprint under Rekindle's inputs.
fn side(identity: &[u8; 32]) -> [u8; 64] {
    let mut key = [0u8; ENCODED_EC_LEN];
    key[0] = ED25519_TYPE;
    key[1..].copy_from_slice(identity);
    iterated(ITERATIONS, identity, &key)
}

/// The iterated SHA-512 of one side: `H1` over the version, key, id and
/// key, then `iterations - 1` more over the previous hash and the key.
fn iterated(iterations: u32, stable_id: &[u8], key: &[u8]) -> [u8; 64] {
    let mut hash = [0u8; 64];
    hash.copy_from_slice(
        &Sha512::new()
            .chain_update([0u8, 0u8])
            .chain_update(key)
            .chain_update(stable_id)
            .chain_update(key)
            .finalize(),
    );
    for _ in 1..iterations {
        let next = Sha512::new()
            .chain_update(hash)
            .chain_update(key)
            .finalize();
        hash.copy_from_slice(&next);
    }
    hash
}

/// One side's 30 digits: six 5-byte big-endian chunks, each mod 100000.
fn side_digits(fingerprint: &[u8; 64]) -> String {
    use std::fmt::Write;
    fingerprint[..DIGITS_PER_SIDE].chunks_exact(5).fold(
        String::with_capacity(DIGITS_PER_SIDE),
        |mut out, chunk| {
            let value = chunk.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
            write!(out, "{:05}", value % 100_000).expect("writing to a String");
            out
        },
    )
}

/// Both sides' digits, the lower string first.
fn joined(local: &[u8; 64], remote: &[u8; 64]) -> String {
    let (ours, theirs) = (side_digits(local), side_digits(remote));
    if ours < theirs {
        ours + &theirs
    } else {
        theirs + &ours
    }
}

/// The scannable form's protobuf encoding (field 1 `version`, fields 2
/// and 3 a nested message holding 32 bytes in its field 1), in field
/// order.
fn scannable(version: u32, local: &[u8; 64], remote: &[u8; 64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SCANNABLE_LEN + 4);
    out.push(0x08);
    let mut value = version;
    while value >= 0x80 {
        out.push(u8::try_from(value & 0x7f).expect("seven bits") | 0x80);
        value >>= 7;
    }
    out.push(u8::try_from(value).expect("below 0x80"));
    for (field, half) in [(0x12u8, local), (0x1a, remote)] {
        let content = u8::try_from(SCANNABLE_BYTES).expect("32 fits a byte");
        out.extend_from_slice(&[field, content + 2, 0x0a, content]);
        out.extend_from_slice(&half[..SCANNABLE_BYTES]);
    }
    out
}

/// Parse [`scannable`]'s canonical form; anything else is malformed.
fn parse_scannable(bytes: &[u8]) -> Result<(u32, &[u8], &[u8]), FingerprintError> {
    let (&tag, mut rest) = bytes.split_first().ok_or(FingerprintError::Malformed)?;
    if tag != 0x08 {
        return Err(FingerprintError::Malformed);
    }
    let mut version = 0u32;
    for shift in (0..35).step_by(7) {
        let (&byte, tail) = rest.split_first().ok_or(FingerprintError::Malformed)?;
        rest = tail;
        version |= u32::from(byte & 0x7f)
            .checked_shl(shift)
            .ok_or(FingerprintError::Malformed)?;
        if byte & 0x80 == 0 {
            break;
        }
    }
    let local = half(&mut rest, 0x12)?;
    let remote = half(&mut rest, 0x1a)?;
    if !rest.is_empty() {
        return Err(FingerprintError::Malformed);
    }
    Ok((version, local, remote))
}

/// One nested half field holding exactly 32 content bytes.
fn half<'a>(rest: &mut &'a [u8], field: u8) -> Result<&'a [u8], FingerprintError> {
    let content = u8::try_from(SCANNABLE_BYTES).expect("32 fits a byte");
    let header = [field, content + 2, 0x0a, content];
    let tail = rest
        .strip_prefix(&header[..])
        .filter(|tail| tail.len() >= SCANNABLE_BYTES)
        .ok_or(FingerprintError::Malformed)?;
    let (value, after) = tail.split_at(SCANNABLE_BYTES);
    *rest = after;
    Ok(value)
}

/// Equality that reads every byte, so its time does not depend on where
/// the halves first differ.
fn equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rekindle's known-answer vectors, over Rekindle's real inputs: the
    // key is `0x06 ‖ Ed25519 key` and the stable id is the Ed25519 key.
    // The two identities are the public keys of RFC 8032 §7.1 TEST 1
    // (Alice) and TEST 2 (Bob). The expected values were computed outside
    // this crate, with OpenSSL and Python, and can be re-derived so:
    //
    //   side() {  # $1 = identity hex; prints the 64-byte fingerprint hex
    //     id=$1; key=06$id
    //     printf '0000%s%s%s' "$key" "$id" "$key" | xxd -r -p \
    //       | openssl dgst -sha512 -binary > h
    //     printf '%s' "$key" | xxd -r -p > k
    //     i=1; while [ $i -lt 5200 ]; do
    //       cat h k | openssl dgst -sha512 -binary > h2; mv h2 h; i=$((i+1))
    //     done
    //     xxd -p -c 64 h
    //   }
    //   echo "$(side $ALICE_IDENTITY) $(side $BOB_IDENTITY)" > sides
    //
    //   # python3, once:
    //   a, b = (bytes.fromhex(x) for x in open('sides').read().split())
    //   d = lambda f: ''.join('%05d' % (int.from_bytes(f[i:i+5], 'big')
    //                                   % 100000) for i in range(0, 30, 5))
    //   print(''.join(sorted([d(a), d(b)])))                # DISPLAYABLE
    //   s = lambda l, r: (bytes([8, 2, 0x12, 34, 0x0a, 32]) + l[:32]
    //                     + bytes([0x1a, 34, 0x0a, 32]) + r[:32]).hex()
    //   print(s(a, b), s(b, a))           # ALICE_SCANNABLE, BOB_SCANNABLE
    const ALICE_IDENTITY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const BOB_IDENTITY: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";
    const ALICE_SIDE: &str = "305db456ef629414ef56c3ea095e1b51f4962b07bdc42f1579b1829c5717b659da40bbfee27b64c5c91f2eef694a28b4377b41cbe3825f3d75ad699736c40a14";
    const BOB_SIDE: &str = "ba5ad2f8c36c259c4da3438442665e513e508c675397778dce6985af080c0da9072771195d9de10486201bf5a0758da19dcf9e5a7b65e13477482cc9bda58a69";
    const DISPLAYABLE: &str = "300319496605179330953896995383927396845152926171918027006668";
    const ALICE_SCANNABLE: &str = "080212220a20305db456ef629414ef56c3ea095e1b51f4962b07bdc42f1579b1829c5717b6591a220a20ba5ad2f8c36c259c4da3438442665e513e508c675397778dce6985af080c0da9";
    const BOB_SCANNABLE: &str = "080212220a20ba5ad2f8c36c259c4da3438442665e513e508c675397778dce6985af080c0da91a220a20305db456ef629414ef56c3ea095e1b51f4962b07bdc42f1579b1829c5717b659";

    fn identity(hex_key: &str) -> [u8; 32] {
        hex::decode(hex_key).unwrap().try_into().unwrap()
    }

    fn vector_sides() -> ([u8; 64], [u8; 64]) {
        (
            side(&identity(ALICE_IDENTITY)),
            side(&identity(BOB_IDENTITY)),
        )
    }

    #[test]
    fn the_known_answer_vectors_reproduce() {
        let (alice, bob) = vector_sides();
        assert_eq!(hex::encode(alice), ALICE_SIDE);
        assert_eq!(hex::encode(bob), BOB_SIDE);
        let (alice_key, bob_key) = (identity(ALICE_IDENTITY), identity(BOB_IDENTITY));
        let alice_view = SafetyNumber::new(&alice_key, &bob_key);
        let bob_view = SafetyNumber::new(&bob_key, &alice_key);
        assert_eq!(alice_view.digits(), DISPLAYABLE);
        assert_eq!(bob_view.digits(), DISPLAYABLE);
        assert_eq!(hex::encode(alice_view.scannable()), ALICE_SCANNABLE);
        assert_eq!(hex::encode(bob_view.scannable()), BOB_SCANNABLE);
        assert_eq!(alice_view.matches_scanned(&bob_view.scannable()), Ok(true));
    }

    #[test]
    fn the_scannable_encoding_is_in_field_order() {
        let (mut l, mut r) = ([0u8; 64], [0u8; 64]);
        l[..32].fill(0x5a);
        r[..32].fill(0xc3);
        l[32..].fill(0xff);
        let expected = format!("080212220a20{}1a220a20{}", "5a".repeat(32), "c3".repeat(32));
        assert_eq!(hex::encode(scannable(2, &l, &r)), expected);
        assert_eq!(scannable(2, &l, &r).len(), SCANNABLE_LEN);
    }

    #[test]
    fn both_peers_see_the_same_digits_and_each_scan_matches_crossed() {
        let (alice, bob) = ([1u8; 32], [2u8; 32]);
        let ours = SafetyNumber::new(&alice, &bob);
        let theirs = SafetyNumber::new(&bob, &alice);
        assert_eq!(ours.digits(), theirs.digits());
        assert_eq!(ours.digits().len(), 2 * DIGITS_PER_SIDE);
        assert!(ours.digits().bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(ours.matches_scanned(&theirs.scannable()), Ok(true));
        assert_eq!(theirs.matches_scanned(&ours.scannable()), Ok(true));
        assert_eq!(ours.matches_scanned(&ours.scannable()), Ok(false));
    }

    #[test]
    fn a_substituted_key_changes_the_number_and_fails_the_scan() {
        let (alice, bob, mallory) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        let alice_view = SafetyNumber::new(&alice, &mallory);
        let bob_view = SafetyNumber::new(&bob, &alice);
        assert_ne!(alice_view.digits(), bob_view.digits());
        assert_eq!(alice_view.matches_scanned(&bob_view.scannable()), Ok(false));
    }

    #[test]
    fn a_malformed_or_other_version_code_is_refused() {
        let number = SafetyNumber::new(&[1; 32], &[2; 32]);
        let code = SafetyNumber::new(&[2; 32], &[1; 32]).scannable();
        let mut v1 = code.clone();
        v1[1] = 1;
        assert_eq!(
            number.matches_scanned(&v1),
            Err(FingerprintError::VersionMismatch { theirs: 1, ours: 2 })
        );
        assert_eq!(
            number.matches_scanned(&code[..code.len() - 1]),
            Err(FingerprintError::Malformed)
        );
        let mut trailing = code.clone();
        trailing.push(0);
        assert_eq!(
            number.matches_scanned(&trailing),
            Err(FingerprintError::Malformed)
        );
        assert_eq!(
            number.matches_scanned(&[]),
            Err(FingerprintError::Malformed)
        );
        let (l, r) = vector_sides();
        let wide = scannable(300, &l, &r);
        assert_eq!(parse_scannable(&wide).map(|p| p.0), Ok(300));
    }
}
