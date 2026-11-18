//! Our own channel message edit and delete reach our own frontends
//! (plan E7.13 M45): the change goes to the mesh, which never echoes a
//! frame to its sender, so the Host publishes the `Edited` / `Deleted`
//! event a receiver gets; without it the author's window kept the old
//! body (walk 8: M45 and the rows after it).

use rekindle_types::channel_keys::KeyScope;
use rekindle_types::subscription_events::{ChannelMessageEvent, StreamFrame, SubscriptionEvent};

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::governance::channel_messages::MessageChange;

const EDITED_CHANNEL: &str = "fedcba9876543210fedcba9876543210";

/// The channel-message events published within a short wait.
async fn channel_events(stream: &mut rekindle_events::EventStream) -> Vec<ChannelMessageEvent> {
    let mut out = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await
    {
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::ChannelMessage(e) = s.event {
                out.push(e);
            }
        }
    }
    out
}

#[tokio::test]
async fn our_edit_and_delete_reach_our_own_frontends() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let key = rekindle_transport::crypto::mek::Mek::from_bytes([7; 32], 1);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, key);
    let mut events = host.ctx.journal.subscribe(None);
    let governance = host.governance();

    let edit = MessageChange::Edit {
        body: "edited".into(),
    };
    governance
        .change_channel_message("c1", EDITED_CHANNEL, "m1", edit)
        .await
        .expect("the edit is sealed and sent");
    governance
        .change_channel_message("c1", EDITED_CHANNEL, "m1", MessageChange::Delete)
        .await
        .expect("the delete is sent");

    let got = channel_events(&mut events).await;
    assert!(
        matches!(&got[..], [
            ChannelMessageEvent::Edited { community, channel, message_id, body: Some(body), .. },
            ChannelMessageEvent::Deleted { community: c2, channel: ch2, message_id: m2 },
        ] if community == "c1" && channel == EDITED_CHANNEL && message_id == "m1" && body == "edited"
            && c2 == "c1" && ch2 == EDITED_CHANNEL && m2 == "m1"),
        "our own edit, then our delete, as a receiver sees them: {got:?}"
    );
    host.shutdown().await.unwrap();
}
