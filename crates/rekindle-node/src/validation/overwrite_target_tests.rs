//! A channel overwrite's target is the canonical role or member key
//! (`rekindle_governance::overwrite::OverwriteTarget`); anything else is
//! refused at the IPC boundary.

use super::*;
use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest};

const HEX32: &str = "0123456789abcdef0123456789abcdef";
const HEX64: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";

fn rejected(request: &IpcRequest) -> bool {
    matches!(
        validate_request(request),
        Err(IpcResponse::Error { code: 400, .. })
    )
}

#[test]
fn an_overwrite_target_is_a_canonical_role_or_member_key() {
    let set = |target_type: &str, target_id: &str| {
        IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelOverwriteSet {
                community: "c".into(),
                channel_id: HEX32.into(),
                target_type: target_type.into(),
                target_id: target_id.into(),
                allow: 0,
                deny: 1,
            },
        ))
    };
    assert!(!rejected(&set("role", HEX32)));
    assert!(!rejected(&set("role", &"0".repeat(32))), "@everyone");
    assert!(!rejected(&set("member", HEX64)));
    for (target_type, target_id) in [
        (
            "role",
            "RoleId([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])".to_owned(),
        ),
        ("role", "everyone".to_owned()),
        ("role", HEX32.to_uppercase()),
        ("role", HEX64.to_owned()),
        ("member", HEX32.to_owned()),
        ("member", String::new()),
        ("everyone", HEX32.to_owned()),
    ] {
        assert!(
            rejected(&set(target_type, &target_id)),
            "{target_type} {target_id}"
        );
    }
}
