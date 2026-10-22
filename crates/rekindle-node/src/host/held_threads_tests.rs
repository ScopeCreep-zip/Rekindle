//! A thread held at receipt stays held across a lock and unlock (plan
//! E3.1 F3h): the hold is stored (`repo::held_threads`) and the unlock
//! loader applies it to the governance it re-merges from the cache.

use rekindle_governance::state::{GovernanceState, ThreadState, TimeoutState};
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey, ThreadId};

use super::{create_ann, offline_host, seed_registry_community};

const THREAD: ThreadId = ThreadId([2; 16]);

fn held_and_merged(host: &crate::host::Host) -> (bool, bool) {
    let communities = host.ctx.communities.map().read();
    let c1 = communities.get("c1").expect("c1 is loaded");
    let merged = c1
        .governance_state
        .as_ref()
        .is_some_and(|gov| gov.threads.contains_key(&THREAD));
    (c1.held_threads.contains(&THREAD), merged)
}

#[tokio::test]
async fn a_held_thread_stays_held_across_a_lock_and_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let late = PseudonymKey([0xbb; 32]);
    let channel = ChannelId([1; 16]);
    let created = GovernanceEntry::ThreadCreated {
        thread_id: THREAD,
        parent_channel_id: channel,
        name: "side".into(),
        thread_type: "public".into(),
        record_key: None,
        invited: Vec::new(),
        forum_tag: None,
        auto_archive_seconds: 3600,
        lamport: 2,
    };
    let (owner, author) = (ann.public_key.clone(), late.clone());
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::governance_cache::save(
                conn,
                &owner,
                "c1",
                &[(author, vec![created])],
                1,
            )
        })
        .await
        .unwrap();

    let mut merged = GovernanceState::default();
    merged.timeouts.insert(
        late.clone(),
        TimeoutState {
            duration_seconds: 3600,
            started_at: host.ctx.clock.now_secs(),
            lamport: 1,
        },
    );
    merged.threads.insert(
        THREAD,
        ThreadState {
            parent_channel_id: channel,
            name: "side".into(),
            thread_type: "public".into(),
            record_key: None,
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 3600,
            creator: late,
            created_lamport: 2,
            archived_lamport: None,
        },
    );
    host.governance().set_governance_state("c1", merged);
    assert_eq!(held_and_merged(&host), (true, false), "held at receipt");

    let lock = host.call(Caller::shell(), IpcRequest::Lock).await;
    assert!(matches!(lock, IpcResponse::Ok { .. }), "{lock:?}");
    let unlock = host
        .call(
            Caller::shell(),
            IpcRequest::Unlock {
                identity: ann.public_key,
                passphrase: "pass".into(),
            },
        )
        .await;
    assert!(matches!(unlock, IpcResponse::Ok { .. }), "{unlock:?}");
    assert_eq!(
        held_and_merged(&host),
        (true, false),
        "still held after the unlock"
    );
    host.shutdown().await.expect("the host stops");
}
