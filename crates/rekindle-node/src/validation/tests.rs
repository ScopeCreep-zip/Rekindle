//! `validate_request` against every rule it enforces.

use super::*;

const B64: &str = "um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
const HEX64: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";
const HEX32: &str = "0123456789abcdef0123456789abcdef";

fn status_of(request: &IpcRequest) -> Option<u32> {
    match validate_request(request) {
        Ok(()) => None,
        Err(IpcResponse::Error { code, .. }) => Some(code),
        Err(other) => panic!("validation answered {other:?}"),
    }
}

fn rejected(request: &IpcRequest) -> bool {
    status_of(request) == Some(400)
}

#[test]
fn short_and_multibyte_keys_are_400_not_a_panic() {
    for key in ["abc", "", "é", &"é".repeat(32), &HEX64.to_uppercase()] {
        assert!(rejected(&IpcRequest::FriendRemove {
            public_key: key.to_owned()
        }));
        assert!(rejected(&IpcRequest::DmTyping {
            peer_key: key.to_owned(),
            typing: true
        }));
    }
    assert_eq!(
        status_of(&IpcRequest::FriendRemove {
            public_key: HEX64.to_owned()
        }),
        None
    );
}

#[test]
fn friend_add_needs_a_record_key_and_a_bounded_note() {
    let target = format!("VLD0:{B64}");
    assert!(!rejected(&IpcRequest::FriendAdd {
        target: target.clone(),
        message: "hi\nthere".into()
    }));
    assert!(rejected(&IpcRequest::FriendAdd {
        target: "VLD0:short".into(),
        message: String::new()
    }));
    assert!(rejected(&IpcRequest::FriendAdd {
        target,
        message: "a".repeat(MAX_NOTE_LEN + 1)
    }));
}

#[test]
fn community_refs_take_a_key_or_a_name() {
    for ok in [
        format!("VLD0:{B64}"),
        "My Community".into(),
        "日本語".into(),
    ] {
        assert!(!rejected(&IpcRequest::ChannelList { community: ok }));
    }
    for bad in [
        "VLD0:abc".to_owned(),
        String::new(),
        "   ".into(),
        "a\x1bb".into(),
        "a".repeat(MAX_NAME_LEN + 1),
    ] {
        assert!(rejected(&IpcRequest::ChannelList { community: bad }));
    }
}

#[test]
fn channel_send_checks_ref_and_body() {
    let send = |channel: &str, body: &str| IpcRequest::ChannelSend {
        community: "c".into(),
        channel: channel.into(),
        body: body.into(),
        reply_to: None,
    };
    assert!(!rejected(&send(HEX32, "hello")));
    assert!(!rejected(&send("general", "hello")));
    assert!(rejected(&send("general", "")));
    assert!(rejected(&send("general", "  \n ")));
    assert!(rejected(&send("general", &"a".repeat(MAX_MESSAGE_LEN + 1))));
    assert!(!rejected(&send("general", &"é".repeat(MAX_MESSAGE_LEN))));
    assert!(rejected(&send("general", "bell\x07")));
    assert!(rejected(&send("", "hello")));
}

#[test]
fn pseudonyms_and_ids_are_exact_hex() {
    assert!(!rejected(&IpcRequest::Kick {
        community: "c".into(),
        target_pseudonym: HEX64.into()
    }));
    assert!(rejected(&IpcRequest::Kick {
        community: "c".into(),
        target_pseudonym: "abcdef".into()
    }));
    assert!(!rejected(&IpcRequest::VoiceJoin {
        community: "c".into(),
        channel: HEX32.into(),
        muted: false,
        deafened: false
    }));
    assert!(rejected(&IpcRequest::VoiceJoin {
        community: "c".into(),
        channel: "general".into(),
        muted: false,
        deafened: false
    }));
    assert!(rejected(&IpcRequest::InviteRevoke {
        community: "c".into(),
        invite_code: "raw-code".into()
    }));
}

#[test]
fn key_scope_is_empty_or_a_channel_id() {
    let rotate = |channel: &str| IpcRequest::MekRotate {
        community: "c".into(),
        channel: channel.into(),
    };
    assert!(!rejected(&rotate("")));
    assert!(!rejected(&rotate(HEX32)));
    assert!(rejected(&rotate("general")));
}

#[test]
fn numeric_bounds() {
    let history = |limit| IpcRequest::ChannelHistory {
        community: "c".into(),
        channel: "general".into(),
        limit,
    };
    assert!(rejected(&history(0)));
    assert!(!rejected(&history(MAX_PAGE)));
    assert!(rejected(&history(MAX_PAGE + 1)));
    let timeout = |duration_seconds| IpcRequest::Timeout {
        community: "c".into(),
        target_pseudonym: HEX64.into(),
        duration_seconds,
        reason: None,
    };
    assert!(rejected(&timeout(0)));
    assert!(!rejected(&timeout(MAX_TIMEOUT_SECONDS)));
    assert!(rejected(&timeout(MAX_TIMEOUT_SECONDS + 1)));
    let create = |slowmode_seconds| IpcRequest::ChannelCreate {
        community: "c".into(),
        name: "general".into(),
        kind: "text".into(),
        category: None,
        topic: Some("about\nthings".into()),
        slowmode_seconds,
    };
    assert!(!rejected(&create(MAX_SLOWMODE_SECONDS)));
    assert!(rejected(&create(MAX_SLOWMODE_SECONDS + 1)));
}

#[test]
fn closed_vocabularies() {
    let presence = |status: &str| IpcRequest::PresenceSet {
        status: status.into(),
        message: None,
    };
    for ok in ["online", "away", "busy", "invisible"] {
        assert!(!rejected(&presence(ok)));
    }
    assert!(rejected(&presence("unknown")));
    let create = |kind: &str| IpcRequest::ChannelCreate {
        community: "c".into(),
        name: "general".into(),
        kind: kind.into(),
        category: None,
        topic: None,
        slowmode_seconds: 0,
    };
    assert!(!rejected(&create("voice")));
    assert!(rejected(&create("chat")));
}

#[test]
fn status_and_description_caps() {
    let presence = |message: String| IpcRequest::PresenceSet {
        status: "online".into(),
        message: Some(message),
    };
    assert!(!rejected(&presence("é".repeat(MAX_STATUS_LEN))));
    assert!(rejected(&presence("a".repeat(MAX_STATUS_LEN + 1))));
    let create = |description: String| IpcRequest::CommunityCreate {
        name: "Rekindle".into(),
        description,
        approval_required: false,
    };
    assert!(!rejected(&create("a".repeat(MAX_DESCRIPTION_LEN))));
    assert!(rejected(&create("a".repeat(MAX_DESCRIPTION_LEN + 1))));
}

#[test]
fn confirmations_are_exact() {
    assert!(!rejected(&IpcRequest::IdentityWipe {
        confirmation: WIPE_CONFIRMATION.into()
    }));
    assert!(rejected(&IpcRequest::IdentityWipe {
        confirmation: "wipe all data".into()
    }));
    assert!(rejected(&IpcRequest::IdentityDestroy {
        confirmation: WIPE_CONFIRMATION.into()
    }));
}

#[test]
fn display_names() {
    let create = |name: &str| IpcRequest::IdentityCreate {
        display_name: name.into(),
    };
    assert!(!rejected(&create("  alice  ")));
    assert!(!rejected(&create(&"日".repeat(
        rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN
    ))));
    assert!(rejected(&create(&"a".repeat(
        rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN + 1
    ))));
    assert!(rejected(&create("")));
    assert!(rejected(&create("hello\x00world")));
}

#[test]
fn join_requires_a_full_invite_link() {
    assert!(rejected(&IpcRequest::CommunityJoin {
        invite: format!("VLD0:{B64}")
    }));
}

#[test]
fn agents() {
    assert!(!rejected(&IpcRequest::AgentRegister {
        name: "rekindle-tui".into(),
        agent_type: rekindle_ipc::message::AgentType::Bot,
        capabilities: vec!["chat".into()],
    }));
    assert!(rejected(&IpcRequest::AgentRevoke {
        name: "../escape".into()
    }));
    assert!(rejected(&IpcRequest::AgentRegister {
        name: "bot".into(),
        agent_type: rekindle_ipc::message::AgentType::Bot,
        capabilities: vec!["chat".into(), "chat".into()],
    }));
    assert!(rejected(&IpcRequest::AgentRegister {
        name: "bot".into(),
        agent_type: rekindle_ipc::message::AgentType::Bot,
        capabilities: vec!["has space".into()],
    }));
}
