use super::*;

#[test]
fn from_str_round_trip() {
    for level in [
        NotificationLevel::All,
        NotificationLevel::Mentions,
        NotificationLevel::Nothing,
    ] {
        let parsed: NotificationLevel = level.as_str().parse().expect("known level");
        assert_eq!(parsed, level);
    }
}

#[test]
fn from_str_unknown_returns_err() {
    let parsed: Result<NotificationLevel, _> = "loud".parse();
    assert!(parsed.is_err());
}

#[test]
fn parse_rejects_unknown_with_error() {
    let err = parse_notification_level("loud").unwrap_err();
    assert!(err.contains("all"), "error mentions valid options");
}

#[test]
fn to_db_is_stable() {
    assert_eq!(NotificationLevel::All.to_db(), 0);
    assert_eq!(NotificationLevel::Mentions.to_db(), 1);
    assert_eq!(NotificationLevel::Nothing.to_db(), 2);
}
