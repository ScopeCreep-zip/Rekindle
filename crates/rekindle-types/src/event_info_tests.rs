use super::*;

#[test]
fn event_info_roundtrip() {
    let e = EventInfo {
        id: "evt_01".into(),
        title: "raid night".into(),
        description: "bring potions".into(),
        creator_pseudonym: "abcd".into(),
        start_time: 1_000,
        end_time: Some(2_000),
        channel_id: Some("ch_01".into()),
        max_attendees: Some(40),
        created_at: 500,
        status: "scheduled".into(),
        rsvps: vec![EventRsvp {
            pseudonym_key: "abcd".into(),
            status: "going".into(),
        }],
        cover_image_ref: Some("blake3:abcdef".into()),
        recurrence: None,
        location: None,
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: EventInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(e, back);
}
