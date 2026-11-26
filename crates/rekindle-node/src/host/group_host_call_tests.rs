//! A skin's call button through `HostApi::call` (plan E4.1 GV2c), over the
//! same `reads::load` → `actions::resolve` path the dispatcher runs: a
//! group window's `hostVoiceChat` (Symbiosis `GroupChatWindow`'s
//! `ChatButtonCall`) opens the Start Group Call panel prefilled with the
//! group's roster, and a 1:1 chat window's still places a 1:1 call.

use rekindle_dm::group::group_id_hex;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::dto::skin_action::SkinAction as A;
use rekindle_types::dto::skin_run::{CallOp, GroupCallInvitee, ShellOp, SkinRun, SkinSelection};

use super::adapters::friends::HostFriends;
use super::adapters::group_dm::HostGroupDm;
use super::direct_tests::ann;
use super::group_dm_tests::member;
use super::Host;

const GID: [u8; 16] = [9; 16];

fn bob() -> String {
    "b".repeat(64)
}

/// `action` pressed in `window` with `selection` set first, as the shell
/// sends them; the run the Host answers with.
async fn press(
    host: &Host,
    window: &str,
    selection: SkinSelection,
    action: A,
) -> serde_json::Value {
    let set = IpcRequest::SkinSelectionSet {
        window: window.to_owned(),
        selection: Some(selection),
    };
    assert!(matches!(
        host.call(Caller::shell(), set).await,
        IpcResponse::Ok { .. }
    ));
    let pressed = IpcRequest::RunSkinAction {
        window: window.to_owned(),
        node: "ChatButtonCall".into(),
        action,
    };
    match host.call(Caller::shell(), pressed).await {
        IpcResponse::Ok { data, .. } => data,
        other => panic!("{window}: hostVoiceChat refused: {other:?}"),
    }
}

#[tokio::test]
async fn a_group_windows_host_voice_chat_opens_the_prefilled_start_group_call() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let deps = HostGroupDm::open(&HostFriends::of(&host.ctx)).unwrap();
    let others = vec![member(bob(), 'b'), member("c".repeat(64), 'c')];
    let own = member(hex::encode(owner), 'a');
    let chain = format!("VLD0:{}", "Q".repeat(43));
    rekindle_dm::create(&deps, GID, "raid", own, others, &chain)
        .await
        .unwrap();
    let group_id = group_id_hex(&GID);

    let group = SkinSelection::Group {
        group_id: group_id.clone(),
    };
    let run = press(&host, "group-raid", group, A::HostVoiceChat).await;
    let invitee = |identity: String, n: char| GroupCallInvitee {
        pubkey: identity,
        display_name: format!("member {n}"),
    };
    let expected = SkinRun::Shell(ShellOp::StartGroupCall {
        group_id,
        group_name: "raid".into(),
        members: vec![invitee(bob(), 'b'), invitee("c".repeat(64), 'c')],
    });
    assert_eq!(run, serde_json::to_value(&expected).unwrap());

    // A 1:1 chat window's Call (its peer selected) places the 1:1 call.
    let peer = SkinSelection::Friend { public_key: bob() };
    let run = press(&host, "chat-bob", peer, A::HostVoiceChat).await;
    let expected = SkinRun::Shell(ShellOp::Call(CallOp::Voice { friend: bob() }));
    assert_eq!(run, serde_json::to_value(&expected).unwrap());
}
