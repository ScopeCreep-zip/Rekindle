use super::removal_text;

#[test]
fn info_shows_our_unlanded_ops() {
    use serde_json::json;
    assert_eq!(super::publications_text(None), "none");
    assert_eq!(super::publications_text(Some(&json!({}))), "none");
    let states = json!({
        "aa": { "kind": "pending" },
        "bb": { "kind": "failed", "reason": "removed" },
    });
    assert_eq!(
        super::publications_text(Some(&states)),
        "1 pending, 1 failed: removed"
    );
}

#[test]
fn standing_names_who_removed_us() {
    assert_eq!(removal_text(None), "member");
    assert_eq!(removal_text(Some(&serde_json::Value::Null)), "member");
    let kicked = serde_json::json!({ "banned": false, "by": "ab".repeat(32), "byName": null });
    assert!(
        removal_text(Some(&kicked)).starts_with("removed by "),
        "{}",
        removal_text(Some(&kicked))
    );
    let named = serde_json::json!({ "banned": true, "by": "ab".repeat(32), "byName": "Ann" });
    assert_eq!(removal_text(Some(&named)), "banned by Ann");
    let why = serde_json::json!({ "banned": false, "by": "ab".repeat(32), "byName": "Ann", "reason": "spam" });
    assert_eq!(removal_text(Some(&why)), "removed by Ann: spam");
    let nobody = serde_json::json!({ "banned": false, "by": null, "byName": null });
    assert_eq!(removal_text(Some(&nobody)), "removed");
}
