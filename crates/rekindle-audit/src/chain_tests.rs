use super::*;
use serde_json::json;

fn fixture_key() -> Zeroizing<[u8; KEY_LEN]> {
    Zeroizing::new([7u8; KEY_LEN])
}

fn rec(kind: AuditKind, n: u64) -> AuditRecord {
    AuditRecord {
        at_ms: 1_700_000_000_000 + i64::try_from(n).unwrap(),
        actor_pub: format!("actor-{n}"),
        kind,
        payload: json!({"seq": n}),
    }
}

#[test]
fn append_then_verify_roundtrip() {
    let key = fixture_key();
    let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
    let e1 = chain.append(rec(AuditKind::VaultUnlocked, 1)).unwrap();
    let e2 = chain.append(rec(AuditKind::FriendAdded, 2)).unwrap();
    let e3 = chain.append(rec(AuditKind::IdentityRotated, 3)).unwrap();
    assert_eq!(e1.cursor, 1);
    assert_eq!(e2.cursor, 2);
    assert_eq!(e3.cursor, 3);
    assert_eq!(e2.prev_mac, e1.mac);
    assert_eq!(e3.prev_mac, e2.mac);
    let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
    verifier
        .verify(&[e1, e2, e3])
        .expect("clean chain verifies");
}

#[test]
fn detects_tampered_payload() {
    let key = fixture_key();
    let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
    let mut entries = vec![
        chain.append(rec(AuditKind::FriendAdded, 1)).unwrap(),
        chain.append(rec(AuditKind::FriendAdded, 2)).unwrap(),
        chain.append(rec(AuditKind::FriendAdded, 3)).unwrap(),
    ];
    // Tamper the middle entry's payload.
    entries[1].record.actor_pub = "evil-actor".into();
    let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
    let err = verifier.verify(&entries).unwrap_err();
    match err {
        VerifyError::MacMismatch { cursor, index } => {
            assert_eq!(cursor, 2);
            assert_eq!(index, 1);
        }
        other => panic!("expected MacMismatch, got {other:?}"),
    }
}

#[test]
fn detects_swapped_entries() {
    let key = fixture_key();
    let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
    let e1 = chain.append(rec(AuditKind::FriendAdded, 1)).unwrap();
    let e2 = chain.append(rec(AuditKind::FriendRemoved, 2)).unwrap();
    // Swap order — prev_mac of e1 (genesis) != e2.mac.
    let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
    let err = verifier.verify(&[e2, e1]).unwrap_err();
    assert!(matches!(
        err,
        VerifyError::PrevMacMismatch { .. } | VerifyError::NonMonotonicCursor { .. }
    ));
}

#[test]
fn detects_inserted_entry() {
    let key = fixture_key();
    let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
    let e1 = chain.append(rec(AuditKind::FriendAdded, 1)).unwrap();
    let e3 = chain.append(rec(AuditKind::FriendAdded, 3)).unwrap();
    // Insert a forged entry between e1 and e3. The forger built it
    // against the previous chain tail (e1.mac, cursor 1), so the
    // forged cursor is 2 — which collides with e3.cursor == 2.
    // Verification catches this via NonMonotonicCursor at e3, or
    // PrevMacMismatch (forged.mac ≠ e3.prev_mac) — either is a
    // valid detection.
    let mut forged_chain = AuditChain::open(fixture_key(), e1.mac, 1);
    let forged = forged_chain
        .append(rec(AuditKind::FriendAdded, 99))
        .unwrap();
    let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
    let err = verifier.verify(&[e1, forged, e3]).unwrap_err();
    assert!(matches!(
        err,
        VerifyError::PrevMacMismatch { .. } | VerifyError::NonMonotonicCursor { .. }
    ));
}

#[test]
fn detects_truncated_genesis() {
    let key = fixture_key();
    let mut chain = AuditChain::open(key.clone(), [0u8; MAC_LEN], 0);
    let _e1 = chain.append(rec(AuditKind::FriendAdded, 1)).unwrap();
    let e2 = chain.append(rec(AuditKind::FriendAdded, 2)).unwrap();
    // Drop e1 — verifier sees e2.prev_mac != [0; 32].
    let verifier = AuditChain::open(key, [0u8; MAC_LEN], 0);
    let err = verifier.verify(&[e2]).unwrap_err();
    assert!(matches!(
        err,
        VerifyError::PrevMacMismatch { cursor: 2, .. }
    ));
}

#[test]
fn wrong_key_fails_every_position() {
    let mut chain = AuditChain::open(fixture_key(), [0u8; MAC_LEN], 0);
    let entries = vec![
        chain.append(rec(AuditKind::FriendAdded, 1)).unwrap(),
        chain.append(rec(AuditKind::FriendAdded, 2)).unwrap(),
    ];
    let wrong_key = Zeroizing::new([99u8; KEY_LEN]);
    let verifier = AuditChain::open(wrong_key, [0u8; MAC_LEN], 0);
    let err = verifier.verify(&entries).unwrap_err();
    // Wrong key invalidates the very first MAC.
    assert!(matches!(
        err,
        VerifyError::MacMismatch {
            cursor: 1,
            index: 0
        }
    ));
}

#[test]
fn cursor_overflow_saturates_safely() {
    // Defence-in-depth: append at u64::MAX should not panic.
    let mut chain = AuditChain::open(fixture_key(), [0u8; MAC_LEN], u64::MAX);
    let entry = chain.append(rec(AuditKind::FriendAdded, 1)).unwrap();
    assert_eq!(entry.cursor, u64::MAX, "saturating_add caps at u64::MAX");
}
