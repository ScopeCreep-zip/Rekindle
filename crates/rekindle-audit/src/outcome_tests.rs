use super::*;
use crate::{AuditKind, AuditRecord};

fn record() -> AuditRecord {
    AuditRecord {
        at_ms: 1,
        actor_pub: "me".into(),
        kind: AuditKind::VaultUnlocked,
        payload: serde_json::json!({}),
    }
}

#[test]
fn a_tampered_entry_is_broken_and_a_removed_tail_is_truncated() {
    let (mut chain, restored) = restore(Zeroizing::new([7; 32]), None, (0, [0; 32]));
    assert_eq!(restored, Restored::Intact);
    let first = chain.append(record()).unwrap();
    let second = chain.append(record()).unwrap();
    assert_eq!(
        verify(&chain, &[first.clone(), second.clone()]),
        Verified::Intact { length: 2 }
    );
    let mut forged = second.clone();
    forged.record.payload = serde_json::json!({ "forged": true });
    assert!(matches!(
        verify(&chain, &[first.clone(), forged]),
        Verified::Broken { at: Some(_), .. }
    ));
    // The vault anchored the second entry; the store ends at the first.
    let (_, restored) = restore(
        Zeroizing::new([7; 32]),
        Some((second.cursor, second.mac)),
        (first.cursor, first.mac),
    );
    assert_eq!(
        restored,
        Restored::Truncated {
            anchor_cursor: second.cursor
        }
    );
}
