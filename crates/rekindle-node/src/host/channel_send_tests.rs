//! The Host's channel send (plan E3.1 E2b), offline: a member without
//! `SEND_MESSAGES` is refused before anything is stored or written, and a
//! send inside the channel's slowmode is refused.

use rekindle_channel::error::ChannelError;
use rekindle_governance::state::GovernanceState;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::{refusal, HostChannel};
use crate::host::community::state::ChannelInfo;
use crate::host::Host;

const CHANNEL: &str = "0123456789abcdef0123456789abcdef";

/// A text channel in Ann's `c1`, with its record key, its slowmode and our
/// last send in it; the merged governance grants nobody anything.
fn add_channel(host: &Host, slowmode_seconds: Option<u32>, last_send_ms: Option<i64>) {
    let channel: ChannelInfo = serde_json::from_value(serde_json::json!({
        "id": CHANNEL,
        "name": "general",
        "channelType": "text",
        "unreadCount": 0,
        "slowmodeSeconds": slowmode_seconds,
    }))
    .expect("a channel");
    {
        let mut map = host.ctx.communities.map().write();
        let community = map.get_mut("c1").expect("the community");
        community.channels.push(channel);
        (community.channel_log_keys).insert(CHANNEL.into(), "VLD0:channel".into());
        if let Some(at) = last_send_ms {
            community.channel_last_send_at.insert(CHANNEL.into(), at);
        }
    }
    (host.ctx.communities).set_governance_state("c1", GovernanceState::default());
}

fn code(response: &rekindle_ipc::IpcResponse) -> u32 {
    match response {
        rekindle_ipc::IpcResponse::Error { code, .. } => *code,
        other => panic!("not a refusal: {other:?}"),
    }
}

async fn stored_rows(host: &Host, owner: &str) -> usize {
    let owner = owner.to_string();
    let db = host.ctx.db.current().unwrap();
    db.call(move |conn| rekindle_db::repo::messages::channel::page(conn, &owner, CHANNEL, None, 10))
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn a_send_without_send_messages_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    add_channel(&host, None, None);
    let refused = HostChannel::new(&host.ctx)
        .send("c1", CHANNEL, "hello")
        .await
        .unwrap_err();
    assert!(
        matches!(refused, ChannelError::PermissionDenied(_)),
        "{refused:?}"
    );
    assert_eq!(code(&refusal(&refused)), 403);
    assert_eq!(stored_rows(&host, &owner).await, 0, "nothing is stored");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_send_inside_the_slowmode_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let now = host.ctx.clock.now_ms_i64();
    add_channel(&host, Some(30), Some(now - 1_000));
    let channel = HostChannel::new(&host.ctx);
    let refused = rekindle_channel::enforce_slowmode_with_bypass(&channel, "c1", CHANNEL, now)
        .unwrap_err();
    assert!(refused.to_string().contains("slowmode active"), "{refused}");
    assert_eq!(code(&refusal(&refused)), 429);
    let later = now + 31_000;
    assert!(rekindle_channel::enforce_slowmode_with_bypass(&channel, "c1", CHANNEL, later).is_ok());
    host.shutdown().await.unwrap();
}
