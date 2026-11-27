//! The device-key commands' request selection (ADR 0022, plan E3.2 S7d-keys).

#[test]
fn re_enrol_device_cli_selects_the_global_trust_changing_request() {
    use crate::cli::{Cli, Command};
    use clap::Parser as _;
    use rekindle_ipc::protocol::LifecycleRequest;
    use rekindle_types::intent::{kind_of, Classified, IntentKind};

    let cli = Cli::try_parse_from(["rekindle", "community", "re-enrol-device"]).unwrap();
    let Some(Command::Community(command)) = cli.command else {
        panic!("expected a community command");
    };
    let request = super::device_request(&command).unwrap();
    assert!(matches!(
        request,
        LifecycleRequest::CommunityDeviceReEnrol {}
    ));
    assert_eq!(
        kind_of(request.name()),
        Some(Classified::Intent(IntentKind::ReEnrolDevice))
    );
}
