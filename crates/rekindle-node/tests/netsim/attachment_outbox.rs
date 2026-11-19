//! Plan E3.3 slice 7: an attachment message rides its author's channel
//! outbox (`M`/4). Every reader holds it after a reload (the outbox
//! catch-up on unlock), and downloads it from the offer the author's chain
//! carries. A post whose sends failed for the outbox deadline sends on the
//! user's Retry (the regression: the sender deferred every attachment row).

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, PinRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(240);

fn pins(request: PinRequest) -> IpcRequest {
    IpcRequest::Community(rekindle_ipc::protocol::CommunityRequest::Pins(request))
}

async fn upload(community: &Community, dir: &std::path::Path) -> Option<String> {
    let source = dir.join("notes.bin");
    std::fs::write(&source, vec![7u8; 70_000]).expect("the file to attach");
    let channel = community.general_id(0).await;
    let uploaded = community.nodes[0]
        .call(pins(PinRequest::AttachmentUpload {
            community: community.anchor.clone(),
            channel_id: channel,
            path: source.display().to_string(),
        }))
        .await;
    uploaded.ok().and_then(|v| v.as_str().map(str::to_owned))
}

/// Node `i`'s row of the attachment `id`, once it holds one.
async fn row(community: &Community, i: usize, id: &str) -> Option<serde_json::Value> {
    let page = community.history(i, None, 100).await.ok()?;
    page.into_iter()
        .find(|m| m["attachment"]["attachmentId"].as_str() == Some(id))
}

async fn downloads(community: &Community, i: usize, id: &str, dir: &std::path::Path) -> bool {
    let target = dir.join(format!("saved-{i}.bin"));
    let channel = community.general_id(i).await;
    eventually(&community.sim, LIMIT, || async {
        community.nodes[i]
            .call(pins(PinRequest::AttachmentDownload {
                community: community.anchor.clone(),
                channel_id: channel.clone(),
                attachment_id: id.to_owned(),
                path: target.display().to_string(),
            }))
            .await
            .is_ok()
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_attachment_post_reaches_every_reader_after_a_reload() {
    let community = Community::of(3, 233).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let dir = tempfile::tempdir().expect("a folder");
    let id = upload(&community, dir.path()).await.expect("ann attaches");
    let reader = &community.nodes[2];
    let key = reader.public_key.clone().expect("an identity");
    reader.ok(IpcRequest::Lock).await;
    reader
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: crate::harness::PASSPHRASE.into(),
        })
        .await;
    for i in 1..3 {
        let held = eventually(&community.sim, LIMIT, || async {
            row(&community, i, &id).await.is_some()
        })
        .await;
        assert!(held, "member {i} holds the attachment post");
        assert!(
            downloads(&community, i, &id, dir.path()).await,
            "member {i} downloads it from the offer on ann's outbox"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_attachment_post_sends_on_retry() {
    let community = Community::of(2, 235).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let sim = &community.sim;
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    let dir = tempfile::tempdir().expect("a folder");
    // The upload stores and queues its post before its DHT writes; offline
    // the possession write refuses, and the post's sends defer until the
    // outbox deadline fails it.
    let _ = upload(&community, dir.path()).await;
    let horizon = rekindle_outbox::OUTBOX_DEADLINE + Duration::from_secs(3600);
    tokio::time::sleep(horizon).await;
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: false,
    });
    let page = (community.history(0, None, 100).await).expect("ann's history");
    let own = (page.iter())
        .find(|m| m["attachment"].is_object())
        .expect("ann's attachment row");
    assert_eq!(
        own["sendState"], "failed",
        "failed past the outbox deadline"
    );
    let id = own["attachment"]["attachmentId"]
        .as_str()
        .unwrap()
        .to_owned();
    let message = own["serverMessageId"].as_str().unwrap().to_owned();
    community.nodes[0]
        .ok(IpcRequest::OutboxRetry { client_id: message })
        .await;
    let held = eventually(sim, LIMIT, || async {
        row(&community, 1, &id).await.is_some()
    })
    .await;
    assert!(held, "the retried attachment post reaches bea");
}
