use super::*;

#[test]
fn this_binary_is_the_cli_the_frontends_name() {
    assert_eq!(env!("CARGO_BIN_NAME"), rekindle_client::CLI_BIN);
}

#[test]
fn unimplemented_exits_69() {
    let err = anyhow::anyhow!(CliError::Unimplemented("node attach".into()));
    assert_eq!(exit_code(&err), 69);
}

#[test]
fn the_daemons_remediation_wins() {
    let err = anyhow::anyhow!(ClientError::Daemon {
        code: 503,
        message: "resume failed".into(),
        remediation: Some("check the network".into()),
    });
    assert_eq!(remediation(&err).as_deref(), Some("check the network"));
    assert_eq!(exit_code(&err), 4);
}

#[test]
fn local_hints_name_this_binary() {
    let err = anyhow::anyhow!(ClientError::Timeout { secs: 5 });
    assert_eq!(
        remediation(&err).as_deref(),
        Some(concat!("run: ", cmd!("status")))
    );
    assert_eq!(exit_code(&err), 2);
}
