use super::*;
use crate::protocol::{ChannelMessageRequest, CommunityRequest};

#[test]
fn every_capability_round_trips_by_name() {
    for capability in AgentCapability::ALL {
        assert_eq!(
            AgentCapability::parse(capability.as_str()),
            Some(*capability)
        );
    }
    assert_eq!(AgentCapability::parse("chat"), None);
}

#[test]
fn key_material_is_never_delegated() {
    assert_eq!(
        IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::MekRotate {
                community: String::new(),
                channel: String::new()
            }
        ))
        .agent_capability(),
        None
    );
    assert_eq!(
        IpcRequest::IdentityExport {
            passphrase: rekindle_types::passphrase::Passphrase::default()
        }
        .agent_capability(),
        None
    );
    assert_eq!(
        IpcRequest::DmInbox { limit: 10 }.agent_capability(),
        Some(AgentCapability::ImHistory)
    );
}

#[test]
fn a_local_file_is_never_delegated() {
    use crate::protocol::PinRequest;
    let pins = |request| IpcRequest::Community(CommunityRequest::Pins(request));
    let upload = PinRequest::AttachmentUpload {
        community: String::new(),
        channel_id: String::new(),
        path: "/a".into(),
    };
    let download = PinRequest::AttachmentDownload {
        community: String::new(),
        channel_id: String::new(),
        attachment_id: String::new(),
        path: "/a".into(),
    };
    assert_eq!(pins(upload).agent_capability(), None);
    assert_eq!(pins(download).agent_capability(), None);
}
