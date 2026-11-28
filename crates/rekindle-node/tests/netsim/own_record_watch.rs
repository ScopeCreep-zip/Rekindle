//! Plan E3.2 S7d-keys.9 part 2 (ADR 0022 rule 4, D4): each device watches
//! its own `M(p,d)`/2, the one source that sees a copy of its data root
//! write under its key before that copy's ops reach any peer, and the
//! record pool re-arms that watch when its count reaches zero.
//!
//! Person `p` (node 1, device A) has a second device B (node 2, restored
//! and enrolled as `devices::second_device` does). B's data root, device
//! vault and all, is copied to node 3: a clone holding B's device key. The
//! clone writes under that key while its head writes and its app messages
//! are cut; A then retires B's key, which B merges without signing. Only
//! B's own-record watch tells B (the retirement check also runs before
//! each sign, and B signs nothing): when the cut heals, the clone's held
//! head write lands on B's own record, the watch sees it, and B stops
//! signing with `OwnKeyRetired` told once.

use std::time::Duration;

use rekindle_governance_runtime::device_signer;
use rekindle_governance_runtime::unlock_barrier::BarrierPhase;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::{Fault, Writes};
use rekindle_types::dht_layout::member_record::GOV_HEAD;
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use crate::devices::{
    device_status, enrol, restart_as, second_device, self_assign, self_assignable_role, unlock,
    A, B,
};
use crate::harness::{eventually, Community, Events};

const LIMIT: Duration = Duration::from_secs(180);
/// The clone of B: node 3, holding B's data root and device vault.
const CLONE: usize = 3;

/// A scenario cut just before the clone's held write is let through.
struct Cloned {
    community: Community,
    /// B's device key.
    key: [u8; 32],
    /// B's own op record, `M(p,d)`.
    record: String,
    /// B's events since the clone was made.
    events: Events,
    /// B's signed head before A retired its key.
    head: Option<u64>,
}

/// How many times B told the user its key was retired.
fn retired_notices(events: &Events) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Notification(NotificationEvent::SystemAlert { title, .. })
            if title == "Signing stopped")
    })
}

/// B's data root and device vault copied to node 3, which starts over it
/// and unlocks: a clone holding B's device key.
async fn clone_b(community: &mut Community) {
    community.nodes[B].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[B]);
    unlock(community, B).await;
    community.nodes[CLONE].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[CLONE], &backup);
    let p_key = community.nodes[B].public_key.clone().expect("p's identity");
    restart_as(community, CLONE, p_key).await;
    unlock(community, CLONE).await;
}

/// Up to A's retirement of B's key, merged by B, with the clone's write
/// under that key held behind its cut head writes and app messages.
async fn cloned_and_retired(seed: u64) -> Cloned {
    let mut community = Community::create(4, seed).await;
    community.join(A).await;
    let role = self_assignable_role(&community, "copied").await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    second_device(&mut community).await;
    enrol(&community, B).await;
    assert!(
        device_status(&community, B, "enrolled").await,
        "B is enrolled"
    );
    clone_b(&mut community).await;
    let anchor = community.anchor.clone();
    let gov = community.nodes[B].host.governance();
    let key = device_signer::device_op_key(&gov, &anchor, false)
        .expect("B's device key")
        .verifying_key()
        .to_bytes();
    let state = gov.governance_state(&anchor).unwrap_or_default();
    let record = device_signer::own_op_record(&gov, &anchor, &state).expect("B's op record");
    let events = community.nodes[B].events().await;
    let clone_gov = community.nodes[CLONE].host.governance();
    let barrier = clone_gov.unlock_barrier(&anchor);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            barrier.phase() == BarrierPhase::Passed
        })
        .await,
        "the clone's unlock catch-up passes: its writes are signed at once"
    );
    community.sim.fault(Fault::WriteFails {
        node: CLONE,
        of: Writes::Subkey {
            subkey: GOV_HEAD,
            record: None,
        },
    });
    community.sim.fault(Fault::DropFrom {
        node: CLONE,
        per_mille: 1000,
    });
    self_assign(&community, CLONE, &role)
        .await
        .expect("the clone writes under B's key");
    let head = (gov.op_store().signed_here_head(&anchor, &key).await).expect("B's signed head");
    let clone_head = (clone_gov.op_store().signed_here_head(&anchor, &key).await)
        .expect("the clone's signed head");
    assert!(clone_head > head, "the clone signed under B's key");
    // A retires the key once it holds B's endorsement of it (B's seq-0
    // `DeviceKey`); before that A refuses it as not `p`'s.
    let retire = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityDeviceRetire {
            anchor: anchor.clone(),
            device_key: hex::encode(key),
        },
    ));
    assert!(
        eventually(&community.sim, LIMIT, || async {
            community.nodes[A].call(retire.clone()).await.is_ok()
        })
        .await,
        "A retires B's key"
    );
    assert!(
        eventually(&community.sim, LIMIT, || async {
            (gov.governance_state(&anchor)).is_some_and(|s| s.devices.retired_from(&key).is_some())
        })
        .await,
        "B merges A's retirement of its key"
    );
    assert_eq!(retired_notices(&events), 0, "B has not seen the clone yet");
    Cloned {
        community,
        key,
        record,
        events,
        head,
    }
}

/// The cut heals: the clone's held head write lands on B's own record,
/// B's own-record watch sees it, and B stops signing, told once, having
/// signed nothing.
async fn the_clone_write_is_seen(cloned: &Cloned) {
    let community = &cloned.community;
    community.sim.fault(Fault::DropFrom {
        node: CLONE,
        per_mille: 0,
    });
    community.sim.fault(Fault::Heal);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            retired_notices(&cloned.events) > 0
        })
        .await,
        "B's own-record watch sees the clone's write and B stops signing"
    );
    tokio::time::sleep(LIMIT).await;
    assert_eq!(retired_notices(&cloned.events), 1, "told once");
    let gov = community.nodes[B].host.governance();
    let head = (gov.op_store())
        .signed_here_head(&community.anchor, &cloned.key)
        .await
        .expect("B's signed head");
    assert_eq!(head, cloned.head, "B signed nothing first");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_own_record_watch_sees_a_clone_write() {
    let cloned = cloned_and_retired(2311).await;
    the_clone_write_is_seen(&cloned).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_own_record_watch_re_arms_on_count_zero() {
    let cloned = cloned_and_retired(2312).await;
    let sim = &cloned.community.sim;
    sim.fault(Fault::WatchDeath {
        node: B,
        key: cloned.record.clone(),
    });
    assert!(
        !sim.watched_subkeys(B, &cloned.record).contains(&GOV_HEAD),
        "B's own-record watch died"
    );
    assert!(
        eventually(sim, LIMIT, || async {
            sim.watched_subkeys(B, &cloned.record).contains(&GOV_HEAD)
        })
        .await,
        "the record pool re-arms the dead own-record watch (D4)"
    );
    the_clone_write_is_seen(&cloned).await;
}
