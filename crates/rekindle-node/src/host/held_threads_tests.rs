//! A thread held at receipt stays held across a lock and unlock (plan
//! E3.1 F3h): the hold is stored (`repo::held_threads`) and the unlock
//! loader applies it to the governance it re-merges from the op store.

use rekindle_governance::fixture::stamp;
use rekindle_governance::state::{GovernanceState, ThreadState, TimeoutState};
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey, ThreadId};

use super::{
    create_ann, offline_host, seed_governance, seed_registry_community, signed_chain,
    CREATOR_RECORD,
};

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
    // The thread's author is the creator: nothing but the hold keeps its
    // thread out of the re-merge.
    let key = SigningKey::from_bytes(&[0xbb; 32]);
    let late = PseudonymKey(key.verifying_key().to_bytes());
    let channel = ChannelId([1; 16]);
    let ops = signed_chain(
        "c1",
        &key,
        CREATOR_RECORD,
        vec![
            GovernanceEntry::ChannelCreated {
                channel_id: channel,
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "VLD0:channel".into(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
            },
            GovernanceEntry::ThreadCreated {
                thread_id: THREAD,
                parent_channel_id: channel,
                name: "side".into(),
                thread_type: "public".into(),
                record_key: None,
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 3600,
            },
        ],
    );
    let cert = seed_governance(&host, &ann.public_key, "c1", &key, &ops).await;
    let stored = rekindle_governance_runtime::op_log::verify_held("c1", &ops);
    assert!(
        rekindle_governance::merge::merge(&cert, &stored.ops)
            .threads
            .contains_key(&THREAD),
        "the stored ops merge the thread"
    );

    let mut merged = GovernanceState::default();
    merged.timeouts.insert(
        late.clone(),
        TimeoutState {
            duration_seconds: 3600,
            started_at: host.ctx.clock.now_secs(),
            stamp: stamp(1),
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
            created: stamp(2),
            archived: None,
            unarchived: None,
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
