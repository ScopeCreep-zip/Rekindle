//! Plan E3.1 D4: a community policy the creator writes reaches another
//! member, a member without `MANAGE_COMMUNITY` is refused, and a community
//! picture is cached and read back while a WebP upload is refused.

use std::time::Duration;

use rekindle_ipc::protocol::SettingsRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::CommunityPolicyView;
use rekindle_types::image_kind::ImageKind;

use crate::harness::{eventually, Community, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

async fn policy(node: &SimNode, community: &str) -> CommunityPolicyView {
    let read = node
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunityPolicyGet {
                community: community.into(),
            },
        )))
        .await;
    serde_json::from_value(read).expect("a policy")
}

fn set_policy(community: &str, text: &str, joins: u32) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunityPolicySet {
            community: community.into(),
            policy_text: Some(text.into()),
            max_joins_per_interval: joins,
            join_interval_seconds: 300,
        },
    ))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_policy_one_member_sets_is_seen_by_another() {
    let community = Community::of(2, 211).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.clone();
    let before = policy(&community.nodes[1], &key).await;
    assert_eq!(before.policy_text, None, "the defaults before any policy");

    community.nodes[0].ok(set_policy(&key, "Be kind.", 5)).await;
    let expected = CommunityPolicyView {
        policy_text: Some("Be kind.".into()),
        max_joins_per_interval: 5,
        join_interval_seconds: 300,
    };
    assert_eq!(policy(&community.nodes[0], &key).await, expected);
    let seen = eventually(&community.sim, LIMIT, || async {
        policy(&community.nodes[1], &key).await == expected
    })
    .await;
    assert!(seen, "the other member reads the new policy");

    let refused = community.nodes[1]
        .call(set_policy(&key, "Mine now.", 1))
        .await
        .expect_err("a member without MANAGE_COMMUNITY is refused");
    assert_eq!(refused.code, 403, "{}", refused.message);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_community_picture_is_cached_and_a_webp_is_refused() {
    let community = Community::of(1, 212).await;
    let key = community.governance_key.clone();
    let pixels = vec![200u8; 64 * 64 * 4];
    let picture = rekindle_image::RgbaImage::from_raw(64, 64, pixels).expect("pixels");
    let png = rekindle_image::encode_png(&rekindle_image::DynamicImage::ImageRgba8(picture))
        .expect("a png");
    let hash = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunitySetAvatar {
                community: key.clone(),
                image: png,
                kind: ImageKind::Png,
            },
        )))
        .await;
    let hash = hash.as_str().expect("the picture's hash").to_owned();
    let url = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunityAvatarGet {
                community: key.clone(),
                hash,
            },
        )))
        .await;
    assert!(
        url.as_str()
            .is_some_and(|u| u.starts_with("data:image/webp;base64,")),
        "the cached picture reads back as a WebP data URL"
    );

    let webp = b"RIFF\x10\0\0\0WEBPVP8L\x04\0\0\0\x2f\0\0\0".to_vec();
    let refused = community.nodes[0]
        .call(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunitySetBanner {
                community: key,
                image: webp,
                kind: ImageKind::Png,
            },
        )))
        .await
        .expect_err("a WebP upload is refused");
    assert_eq!(refused.code, 400, "{}", refused.message);
}
