use super::{MediaScope, Track};

#[test]
fn scope_json_is_externally_tagged_camel_case() {
    let call = MediaScope::of_session(None, "ab");
    assert_eq!(
        serde_json::to_value(&call).unwrap(),
        serde_json::json!({ "directCall": { "peer": "ab" } })
    );
    let channel = MediaScope::of_session(Some("c"), "v");
    assert_eq!(
        serde_json::to_value(&channel).unwrap(),
        serde_json::json!({ "channel": { "communityId": "c", "channelId": "v" } })
    );
    assert_eq!(serde_json::to_value(Track::Screen).unwrap(), "screen");
    assert_eq!(Track::ALL.map(Track::label), ["camera", "screen"]);
}
