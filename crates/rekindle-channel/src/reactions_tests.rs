use super::*;

#[test]
fn build_reaction_carries_fields_verbatim() {
    let r = build_reaction("msg_1".into(), "👍".into(), true, 42);
    assert_eq!(r.message_id, "msg_1");
    assert_eq!(r.expression, "👍");
    assert!(r.added);
    assert_eq!(r.lamport, 42);

    let r_remove = build_reaction("msg_2".into(), "❤️".into(), false, 99);
    assert!(!r_remove.added);
    assert_eq!(r_remove.lamport, 99);
}

#[test]
fn build_envelope_added_branch_is_reaction_added() {
    let env = build_reaction_envelope(
        "chan".into(),
        "msg".into(),
        "✨".into(),
        "pseu".into(),
        true,
    );
    match env.into_control() {
        Some(ControlPayload::ReactionAdded {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        }) => {
            assert_eq!(channel_id, "chan");
            assert_eq!(message_id, "msg");
            assert_eq!(emoji, "✨");
            assert_eq!(reactor_pseudonym, "pseu");
        }
        other => panic!("expected ReactionAdded, got {other:?}"),
    }
}

#[test]
fn build_envelope_removed_branch_is_reaction_removed() {
    let env = build_reaction_envelope(
        "chan".into(),
        "msg".into(),
        "🔥".into(),
        "pseu".into(),
        false,
    );
    assert!(matches!(
        env.as_control(),
        Some(ControlPayload::ReactionRemoved { .. })
    ));
}
