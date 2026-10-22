//! Expressions (plan E3.1 E3c): an emoji one member uploads is listed by
//! another member through the governance mesh, and the uploader reads its
//! picture back from the Host's chunk cache.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::ExpressionInfo;
use rekindle_types::image_kind::ImageKind;

use crate::harness::{eventually, Community, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

/// A 1×1 RGBA PNG.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

async fn expressions(node: &SimNode, community: &str) -> Vec<ExpressionInfo> {
    let listed = node
        .ok(IpcRequest::Community(CommunityRequest::Expressions {
            community: community.into(),
        }))
        .await;
    serde_json::from_value(listed).expect("an expression list")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn expression_upload_seen() {
    let community = Community::of(2, 165).await;
    // Every join-time write settles.
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.clone();

    let id = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::EmojiUpload {
            community: key.clone(),
            name: "spark".into(),
            bytes: PNG_1X1.to_vec(),
            kind: ImageKind::Png,
        }))
        .await;
    let id: String = serde_json::from_value(id).expect("an expression id");
    let own = expressions(&community.nodes[0], &key).await;
    let mine = own.iter().find(|e| e.expression_id == id).expect("listed");
    assert_eq!(mine.kind, "emoji");
    assert_eq!(mine.media_type.as_deref(), Some("image/png"));
    assert!(
        (mine.inline_data_url.as_deref()).is_some_and(|u| u.starts_with("data:image/png;base64,")),
        "the uploader reads the picture from its cache"
    );

    let seen = eventually(&community.sim, LIMIT, || async {
        expressions(&community.nodes[1], &key)
            .await
            .iter()
            .any(|e| e.expression_id == id && e.name == "spark")
    })
    .await;
    assert!(seen, "the other member lists the emoji");
}
