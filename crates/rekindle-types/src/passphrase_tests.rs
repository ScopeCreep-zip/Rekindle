use super::*;

#[test]
fn it_is_a_plain_string_on_the_wire_and_redacted_in_logs() {
    let pass = Passphrase::from(rekindle_test_fixtures::PASSPHRASE_SPOKEN);
    let json = serde_json::to_string(&pass).unwrap();
    assert_eq!(
        json,
        format!("\"{}\"", rekindle_test_fixtures::PASSPHRASE_SPOKEN)
    );
    assert_eq!(serde_json::from_str::<Passphrase>(&json).unwrap(), pass);
    assert_eq!(&*pass, rekindle_test_fixtures::PASSPHRASE_SPOKEN);
    assert_eq!(format!("{pass:?}"), "Passphrase(<redacted>)");
}
