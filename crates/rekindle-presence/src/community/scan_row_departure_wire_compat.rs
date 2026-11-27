use super::{parse_and_classify_row, ClassifiedRow};
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::MemberPresence;
use std::collections::HashSet;

fn signed(departed: bool) -> (MemberPresence, Vec<u8>) {
    let signing = SigningKey::from_bytes(&[21u8; 32]);
    let mut presence = MemberPresence {
        pseudonym_key: PseudonymKey(signing.verifying_key().to_bytes()),
        status: if departed { "offline" } else { "online" }.into(),
        departed,
        last_heartbeat: 1_700_000_000,
        ..Default::default()
    };
    let sig = derive::sign_with_pseudonym(&signing, &presence.signing_bytes());
    presence.signature = sig.to_vec();
    let raw = serde_json::to_vec(&presence).expect("serialize");
    (presence, raw)
}

/// Half one: a **live** row must stay byte-identical to the format
/// that predates the field, or every existing member's row starts
/// failing verification on readers that do know about it.
#[test]
fn a_live_row_does_not_carry_the_field() {
    let (_, raw) = signed(false);
    let json = String::from_utf8(raw).expect("utf8");
    assert!(
        !json.contains("departed"),
        "a non-departed row must omit the field entirely: {json}"
    );
}

#[test]
fn a_departed_row_does_carry_it() {
    let (_, raw) = signed(true);
    let json = String::from_utf8(raw).expect("utf8");
    assert!(json.contains("departed"), "departed row must carry it");
}

/// Both rows verify under a reader that knows the field.
#[test]
fn both_rows_verify_for_a_current_reader() {
    let none = HashSet::new();
    let (_, live) = signed(false);
    assert!(matches!(
        parse_and_classify_row(&live, &none, 0, 0),
        ClassifiedRow::Accepted(_)
    ));
    let (_, gone) = signed(true);
    assert_eq!(
        parse_and_classify_row(&gone, &none, 0, 0),
        ClassifiedRow::Departed
    );
}

/// Half two, and the reason this is safe to deploy incrementally.
///
/// Simulates an older reader by stripping the field it would not
/// know before the signature is recomputed — exactly what serde does
/// when it deserialises into a struct without it. The signature no
/// longer covers the same bytes, so the row is rejected, and a
/// rejected row is already reclaimable. An old reader and a new one
/// therefore reach the same conclusion about a departed slot by two
/// different routes, with no flag day between them.
#[test]
fn an_older_reader_rejects_a_departed_row_rather_than_honouring_it() {
    let (_, gone) = signed(true);
    let mut value: serde_json::Value = serde_json::from_slice(&gone).expect("json");
    value
        .as_object_mut()
        .expect("object")
        .remove("departed")
        .expect("field was present");
    let without = serde_json::to_vec(&value).expect("serialize");

    assert_eq!(
        parse_and_classify_row(&without, &HashSet::new(), 0, 0),
        ClassifiedRow::SignatureRejected,
        "dropping the field must invalidate the signature, not silently \
             produce a live-looking member"
    );
}

/// And the inverse: stripping the field from a *live* row changes
/// nothing, because it was never there. This is what makes existing
/// deployments keep working.
#[test]
fn an_older_reader_still_accepts_a_live_row() {
    let (_, live) = signed(false);
    let value: serde_json::Value = serde_json::from_slice(&live).expect("json");
    assert!(
        !value.as_object().expect("object").contains_key("departed"),
        "nothing to strip"
    );
    assert!(matches!(
        parse_and_classify_row(&live, &HashSet::new(), 0, 0),
        ClassifiedRow::Accepted(_)
    ));
}
