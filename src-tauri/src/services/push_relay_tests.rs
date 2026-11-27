use super::validate_push_relay_inputs;

#[test]
fn validate_rejects_short_pseudonym() {
    assert!(validate_push_relay_inputs("abc", "token", "self", &["VLD0:foo".into()]).is_err());
}

#[test]
fn validate_rejects_unknown_platform() {
    let pseudonym = "a".repeat(64);
    assert!(
        validate_push_relay_inputs(&pseudonym, "token", "discord", &["VLD0:foo".into()]).is_err()
    );
}

#[test]
fn validate_rejects_empty_record_keys() {
    let pseudonym = "a".repeat(64);
    assert!(validate_push_relay_inputs(&pseudonym, "token", "self", &[]).is_err());
}

#[test]
fn validate_rejects_empty_token() {
    let pseudonym = "a".repeat(64);
    assert!(validate_push_relay_inputs(&pseudonym, "", "self", &["VLD0:foo".into()]).is_err());
}
