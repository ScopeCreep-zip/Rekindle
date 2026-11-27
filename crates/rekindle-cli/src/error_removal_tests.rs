use super::*;

/// R3-4: the daemon's removal remediation is printed for its 403, not
/// this binary's "run: rekindle unlock".
#[test]
fn a_removal_403_prints_the_removal_remediation() {
    let err = anyhow::anyhow!(ClientError::Daemon {
        code: 403,
        message: "channel send: you were removed from this community".into(),
        remediation: Some("you can rejoin with a new invite, or forget the community".into()),
    });
    let hint = remediation(&err).unwrap();
    assert!(hint.contains("new invite"), "{hint}");
    assert!(!hint.contains("unlock"), "{hint}");
    assert_eq!(exit_code(&err), 3);
}
