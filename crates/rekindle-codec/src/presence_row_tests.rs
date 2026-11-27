use rekindle_types::id::PseudonymKey;

use super::*;

fn signed_row(secret: u8) -> (Vec<u8>, String) {
    let key = rekindle_secrets::derive::derive_community_pseudonym(&[secret; 32], "c1");
    let mut row = MemberPresence {
        pseudonym_key: PseudonymKey(key.verifying_key().to_bytes()),
        ..Default::default()
    };
    row.signature =
        rekindle_secrets::derive::sign_with_pseudonym(&key, &row.signing_bytes()).to_vec();
    (
        serde_json::to_vec(&row).unwrap(),
        hex::encode(row.pseudonym_key.0),
    )
}

#[test]
fn a_departure_row_is_our_signed_tombstone() {
    let key = rekindle_secrets::derive::derive_community_pseudonym(&[3; 32], "c1");
    let row = departure_row(&key, 1_800_000_000);
    let me = hex::encode(key.verifying_key().to_bytes());
    assert_eq!(classify_superseding_row(&row, &me), SupersedingRow::Ours);
    let parsed: MemberPresence = serde_json::from_slice(&row).unwrap();
    assert!(parsed.departed);
    assert_eq!(parsed.status, "offline");
}

#[test]
fn our_row_another_members_row_and_forgeries_are_told_apart() {
    let (ours, me) = signed_row(1);
    let (theirs, them) = signed_row(2);
    assert_eq!(classify_superseding_row(&ours, &me), SupersedingRow::Ours);
    assert_eq!(
        classify_superseding_row(&theirs, &me),
        SupersedingRow::Member(them)
    );
    // Signed content changed after signing (a flipped byte in an ignored
    // key would not count: the signature covers the canonical form).
    let forged = String::from_utf8(ours.clone())
        .unwrap()
        .replacen("\"status\":\"online\"", "\"status\":\"away\"", 1)
        .into_bytes();
    assert_ne!(forged, ours);
    assert!(matches!(
        classify_superseding_row(&forged, &me),
        SupersedingRow::Unverified(_)
    ));
    assert_eq!(
        classify_superseding_row(b"", &me),
        SupersedingRow::Unverified("empty-payload")
    );
}
