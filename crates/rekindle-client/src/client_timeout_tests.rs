use super::*;
use rekindle_ipc::protocol::{ChannelRequest, MemberRequest};

/// A governance write answers once its op landed (plan E3.2 S6b), up to
/// the Host's `NETWORK_ANSWER_DEADLINE`, so its client waits longer.
#[test]
fn governance_writes_wait_for_the_network_answer() {
    let kick = IpcRequest::Community(CommunityRequest::Members(MemberRequest::Kick {
        community: "c".into(),
        target_pseudonym: "p".into(),
        reason: None,
    }));
    let leave = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityLeave { anchor: "c".into() },
    ));
    let list = IpcRequest::Community(CommunityRequest::Channels(ChannelRequest::ChannelList {
        community: "c".into(),
    }));
    assert_eq!(request_timeout(&kick), LONG_TIMEOUT);
    assert_eq!(request_timeout(&leave), LONG_TIMEOUT);
    assert_eq!(request_timeout(&list), DEFAULT_TIMEOUT);
    assert!(rekindle_types::config::NETWORK_ANSWER_DEADLINE < LONG_TIMEOUT);
}
