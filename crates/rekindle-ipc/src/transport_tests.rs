use super::*;

#[test]
fn local_credentials_are_valid() {
    let creds = PeerCredentials::local();
    assert!(creds.pid.is_some_and(|pid| pid > 0));
    #[cfg(unix)]
    assert!(creds.uid < u32::MAX); // Not the sentinel value
}
