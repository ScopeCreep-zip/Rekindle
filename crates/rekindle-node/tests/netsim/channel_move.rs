//! Move channel to category (plan E7.15 S2/S3): an admin's `ChannelMove`
//! is a governance entry like every channel edit, so a second member's
//! channel list shows the channel in that category once the entry reaches
//! it, and a move out of every category (`None`) reaches it the same way.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest};

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

fn channels(request: ChannelRequest) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Channels(request))
}

/// The category node `i` lists the channel `id` in, or `None` when it
/// lists it in none; `Err` while it does not list the channel at all.
async fn category_of(community: &Community, i: usize, id: &str) -> Result<Option<String>, ()> {
    let list = community.nodes[i]
        .ok(channels(ChannelRequest::ChannelList {
            community: community.anchor.clone(),
        }))
        .await;
    let channel = (list.as_array().into_iter().flatten())
        .find(|c| c["id"] == id)
        .ok_or(())?;
    Ok(channel["categoryId"].as_str().map(str::to_owned))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_moved_channel_shows_in_its_category_on_a_second_member() {
    let community = Community::of(2, 232).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let admin = &community.nodes[0];

    let category = admin
        .ok(channels(ChannelRequest::CategoryCreate {
            community: community.anchor.clone(),
            name: "games".into(),
        }))
        .await["categoryId"]
        .as_str()
        .expect("a category id")
        .to_owned();
    let channel = admin
        .ok(channels(ChannelRequest::ChannelCreate {
            community: community.anchor.clone(),
            name: "lobby".into(),
            kind: "text".into(),
            category: None,
            topic: None,
            slowmode_seconds: 0,
            parent_voice_channel: None,
        }))
        .await["id"]
        .as_str()
        .expect("a channel id")
        .to_owned();
    assert!(
        eventually(&community.sim, LIMIT, || async {
            category_of(&community, 1, &channel).await == Ok(None)
        })
        .await,
        "the member lists the new channel outside every category"
    );

    admin
        .ok(channels(ChannelRequest::ChannelMove {
            community: community.anchor.clone(),
            channel_id: channel.clone(),
            category: Some(category.clone()),
        }))
        .await;
    assert_eq!(
        category_of(&community, 0, &channel).await,
        Ok(Some(category.clone())),
        "the admin lists the channel in the category at once"
    );
    assert!(
        eventually(&community.sim, LIMIT, || async {
            category_of(&community, 1, &channel).await == Ok(Some(category.clone()))
        })
        .await,
        "the member lists the channel in the category"
    );

    admin
        .ok(channels(ChannelRequest::ChannelMove {
            community: community.anchor.clone(),
            channel_id: channel.clone(),
            category: None,
        }))
        .await;
    assert!(
        eventually(&community.sim, LIMIT, || async {
            category_of(&community, 1, &channel).await == Ok(None)
        })
        .await,
        "the member lists the channel outside every category again"
    );
}
