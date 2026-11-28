use super::{Cli, Command};
use clap::Parser;

#[test]
fn location_settings_map_to_the_host_read_and_field_write() {
    use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, SettingsRequest};
    use rekindle_types::presence::{PresencePolicyField, ShareScope};

    for (state, scope) in [("on", ShareScope::Members), ("off", ShareScope::None)] {
        let cli = Cli::try_parse_from(["rekindle", "settings", "share-location", "Group", state])
            .unwrap();
        let Some(Command::Settings(command)) = cli.command else {
            panic!("expected settings command");
        };
        assert!(matches!(
            crate::presence::settings_request(&command),
            IpcRequest::Community(CommunityRequest::Settings(
                SettingsRequest::CommunitySetPresencePolicyField {
                    community,
                    field: PresencePolicyField::ShareLocation(actual),
                }
            )) if community == "Group" && actual == scope
        ));
    }

    let cli = Cli::try_parse_from([
        "rekindle",
        "settings",
        "share-location",
        "Group",
        "--format",
        "json",
    ])
    .unwrap();
    let Some(Command::Settings(command)) = cli.command else {
        panic!("expected settings command");
    };
    assert!(matches!(
        crate::presence::settings_request(&command),
        IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunityGetPresencePolicy { community }
        )) if community == "Group"
    ));
}
