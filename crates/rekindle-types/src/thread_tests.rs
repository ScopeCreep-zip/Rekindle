use super::*;

#[test]
fn thread_info_roundtrip() {
    let t = ThreadInfo {
        id: "thr_abcdef".into(),
        channel_id: "ch_123".into(),
        name: "design".into(),
        starter_message_id: "msg_xyz".into(),
        creator_pseudonym: "abcd".into(),
        forum_tag: Some("question".into()),
        created_at: 100,
        archived: false,
        auto_archive_seconds: 86_400,
        last_message_at: 200,
        message_count: 7,
    };
    let json = serde_json::to_string(&t).unwrap();
    let back: ThreadInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(t, back);
}
