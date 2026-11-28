//! Plan E3.3 (walk M47): an edit is an entry in its author's record, stored
//! on the author and on every reader, so both read the edited body after a
//! reload (`get_channel_messages`, which merges the stored rows with the
//! channel record); only the author changes its message (E1), and its delete
//! is final (E3) and never restored by a re-read of the record (E4).

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelMessageRequest, CommunityRequest, IpcRequest};

use crate::harness::{eventually, Community, Refused};

const LIMIT: Duration = Duration::from_secs(180);

async fn change(
    community: &Community,
    i: usize,
    message_id: &str,
    body: Option<&str>,
) -> Result<serde_json::Value, Refused> {
    let channel = community.general_id(i).await;
    let message_id = message_id.to_owned();
    let request = match body {
        Some(body) => ChannelMessageRequest::ChannelMessageEdit {
            channel,
            message_id,
            body: body.into(),
        },
        None => ChannelMessageRequest::ChannelMessageDelete {
            channel,
            message_id,
        },
    };
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Messages(request)))
        .await
}

/// Node `i`'s reload of the general channel: the message `id`'s body and
/// edit time, or `None` when it is not shown.
async fn shown(community: &Community, i: usize, id: &str) -> Option<(String, Option<u64>)> {
    let page = community.history(i, None, 100).await.ok()?;
    page.iter().find(|m| m["serverMessageId"] == id).map(|m| {
        (
            m["body"].as_str().unwrap_or("").to_owned(),
            m["editedAt"].as_u64(),
        )
    })
}

async fn eventually_shown(community: &Community, i: usize, id: &str, want: Option<&str>) -> bool {
    eventually(&community.sim, LIMIT, || async {
        match (shown(community, i, id).await, want) {
            (Some((body, edited)), Some(want)) => body == want && edited.is_some(),
            (None, None) => true,
            _ => false,
        }
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn channel_edit_both_ways() {
    let community = Community::of(2, 215).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let posted = community
        .post(0, "before the edit")
        .await
        .expect("ann posts");
    let id = posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned();
    assert!(
        community.history_holds(1, "before the edit", LIMIT).await,
        "bea reads ann's post"
    );

    change(&community, 0, &id, Some("after the edit"))
        .await
        .expect("ann edits her post");
    assert!(
        eventually_shown(&community, 0, &id, Some("after the edit")).await,
        "ann's reload shows her edit"
    );
    assert!(
        eventually_shown(&community, 1, &id, Some("after the edit")).await,
        "bea's reload shows ann's edit"
    );
    let refused = change(&community, 1, &id, Some("bea's words")).await;
    assert!(
        matches!(&refused, Err(Refused { code: 409, .. })),
        "E1: bea cannot edit ann's post: {refused:?}"
    );

    change(&community, 0, &id, None)
        .await
        .expect("ann deletes her post");
    assert!(
        eventually_shown(&community, 0, &id, None).await,
        "ann's reload no longer shows it"
    );
    assert!(
        eventually_shown(&community, 1, &id, None).await,
        "bea's reload no longer shows it, though the record keeps the post"
    );
    let refused = change(&community, 0, &id, Some("back again")).await;
    assert!(
        matches!(&refused, Err(Refused { code: 409, .. })),
        "E3: an edit after the delete is refused: {refused:?}"
    );
}
