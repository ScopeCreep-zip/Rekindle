//! An unsigned thread frame from the mesh changes no stored thread (plan
//! E3.2 S8-E; threat model: unsigned frames never change state). A
//! thread's row and archive state follow only the signed governance ops
//! (`governance/thread_events.rs`).

use std::sync::Arc;

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_db::repo::threads;
use rekindle_lifecycle::SessionScope;
use rekindle_types::thread::ThreadInfo;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::HostChannel;

fn thread(id: &str) -> ThreadInfo {
    ThreadInfo {
        id: id.into(),
        channel_id: "ch".into(),
        name: "raid night".into(),
        starter_message_id: "m1".into(),
        creator_pseudonym: "aa".into(),
        forum_tag: None,
        created_at: 1,
        archived: false,
        auto_archive_seconds: 3_600,
        last_message_at: 1,
        message_count: 0,
    }
}

#[tokio::test]
async fn a_forged_thread_frame_changes_no_stored_thread() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let db = host.ctx.db.current().unwrap();
    let o = owner.clone();
    db.call(move |c| threads::upsert(c, &o, "c1", &thread("thr_a")))
        .await
        .unwrap();
    let scope = SessionScope::new("test", Arc::new(|_| {}));
    let channel = HostChannel::new(&host.ctx);
    let forged = [
        ControlPayload::ThreadArchived {
            thread_id: "thr_a".into(),
            archived: true,
        },
        ControlPayload::ThreadCreated {
            thread: thread("thr_b"),
        },
    ];
    for payload in &forged {
        let event = channel.on_thread_control("c1", payload, &scope).await;
        assert!(event.is_none(), "{payload:?} raised {event:?}");
    }
    let (a, b) = db
        .call(move |c| {
            Ok((
                threads::get(c, &owner, "c1", "thr_a")?,
                threads::get(c, &owner, "c1", "thr_b")?,
            ))
        })
        .await
        .unwrap();
    assert!(
        !a.expect("thr_a stays stored").archived,
        "archive unchanged"
    );
    assert!(b.is_none(), "no row from an unsigned ThreadCreated");
}
