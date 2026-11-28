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
