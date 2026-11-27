use super::*;
use crate::shell_commands::{HostCall, HOST_CALLS};

/// `community_device_enrol` sends `CommunityDeviceEnrol` for the
/// community it names, and the port ledger records that request.
#[test]
fn community_device_enrol_sends_the_enrol_request() {
    let request = device_enrol_request("anchor-1".to_owned());
    assert!(matches!(
        &request,
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityDeviceEnrol { anchor }
        )) if anchor == "anchor-1"
    ));
    let ledger = HOST_CALLS
        .iter()
        .find(|(command, _)| *command == "community_device_enrol")
        .map(|(_, call)| *call);
    assert_eq!(ledger, Some(HostCall::Request(request.name())));
}
