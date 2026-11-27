use super::ConversationType;

#[test]
fn every_type_round_trips_and_others_are_refused() {
    for t in [
        ConversationType::Dm,
        ConversationType::GroupDm,
        ConversationType::Channel,
    ] {
        assert_eq!(t.as_str().parse::<ConversationType>(), Ok(t));
    }
    assert!("group".parse::<ConversationType>().is_err());
}
