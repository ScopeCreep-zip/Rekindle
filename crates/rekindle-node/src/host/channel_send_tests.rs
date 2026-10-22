//! The Host's channel send (plan E3.1 E2b), offline: a member without
//! `SEND_MESSAGES` is refused before anything is stored or written, and a
//! send inside the channel's slowmode is refused.

use rekindle_channel::error::ChannelError;
use rekindle_governance::state::GovernanceState;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::{refusal, HostChannel};
use crate::host::Host;

const CHANNEL: &str = "0123456789abcdef0123456789abcdef";

/// A text channel in Ann's `c1`, with its record key, its slowmode and our
/// last send in it, stored as a join stores them and loaded into the
/// community store by the unlock's restore (which derives our pseudonym);
/// the merged governance grants nobody anything.
async fn add_channel(
    host: &Host,
    owner: &str,
    slowmode_seconds: Option<u32>,
    last_send_ms: Option<i64>,
) {
    let o = owner.to_string();
    (host.ctx.db.current().unwrap())
        .call(move |conn| {
            rekindle_db::repo::channels::insert(
                conn,
                &o,
                "c1",
                &rekindle_db::repo::channels::StoredChannel {
                    id: CHANNEL,
                    name: "general",
                    channel_type: "text",
                    category_id: None,
                    topic: "",
                    slowmode_seconds: slowmode_seconds.unwrap_or(0),
                    nsfw: false,
                    message_record_key: Some("VLD0:channel"),
                    mek_generation: 0,
                    parent_voice_channel_id: None,
                },
            )?;
            match last_send_ms {
                Some(at) => rekindle_db::repo::channels::set_last_send(conn, &o, "c1", CHANNEL, at),
                None => Ok(()),
            }
        })
        .await
        .unwrap();
    let secret = host.ctx.identity_secret().expect("an unlocked identity");
    crate::host::adapters::governance::loader::restore(&host.ctx, owner, &secret)
        .await
        .unwrap();
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
    add_channel(&host, &owner, None, None).await;
    let refused = HostChannel::new(&host.ctx)
        .send("c1", CHANNEL, "hello", None)
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
    add_channel(&host, &owner, Some(30), Some(now - 1_000)).await;
    let channel = HostChannel::new(&host.ctx);
    let refused =
        rekindle_channel::enforce_slowmode_with_bypass(&channel, "c1", CHANNEL, now).unwrap_err();
    assert!(refused.to_string().contains("slowmode active"), "{refused}");
    assert_eq!(code(&refusal(&refused)), 429);
    let later = now + 31_000;
    assert!(rekindle_channel::enforce_slowmode_with_bypass(&channel, "c1", CHANNEL, later).is_ok());
    host.shutdown().await.unwrap();
}
