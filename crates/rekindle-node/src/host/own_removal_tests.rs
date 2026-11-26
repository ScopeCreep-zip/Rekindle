//! Our own removal follows the merged state (plan E3.2 S9 own removal):
//! a removal the merge later voids (strong removal: concurrent with the
//! remover's own removal) clears the mark, so sends are not refused.

use std::sync::Arc;

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_types::id::PseudonymKey;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::HostChannel;
use crate::host::adapters::governance::HostGovernance;

/// Our pseudonym in `c1`, as a join stores it.
fn set_our_pseudonym(host: &crate::host::Host) -> PseudonymKey {
    let me = PseudonymKey([0xcd; 32]);
    if let Some(c1) = host.ctx.communities.map().write().get_mut("c1") {
        c1.my_pseudonym_key = Some(me.to_hex());
    }
    me
}

/// `me` removed by `by`; with `voided`, the merge kept `me` a member too
/// (the removal's author was removed concurrently, so its act is void).
fn removing(me: PseudonymKey, by: PseudonymKey, voided: bool) -> GovernanceState {
    let mut state = GovernanceState::default();
    state.removed.insert(me.clone(), OpStamp::MIN);
    state.removed_by.insert(me.clone(), by);
    if voided {
        state.members.insert(
            me,
            MemberState {
                member_record: "VLD0:member".into(),
                member_record_owner: String::new(),
                joined: OpStamp::MIN,
            },
        );
    }
    state
}

#[tokio::test]
async fn a_removal_the_merge_voids_stops_refusing_sends() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let (me, bea) = (set_our_pseudonym(&host), PseudonymKey([7; 32]));
    let governance = HostGovernance::new(Arc::clone(&host.ctx));
    let channel = HostChannel::new(&host.ctx);

    governance.set_governance_state("c1", removing(me.clone(), bea.clone(), false));
    let removal = host
        .ctx
        .communities
        .own_removal("c1")
        .expect("marked removed");
    assert_eq!(removal.by, Some(bea.clone()));
    assert!(
        channel.refuse_removed("c1").is_err(),
        "a removed member's send is refused"
    );

    governance.set_governance_state("c1", removing(me, bea, true));
    assert_eq!(
        host.ctx.communities.own_removal("c1"),
        None,
        "the void removal's mark goes"
    );
    assert!(
        channel.refuse_removed("c1").is_ok(),
        "the member sends again"
    );
    host.shutdown().await.unwrap();
}

/// `c1` held as removed by `bea`, with Ann's membership loaded.
async fn removed_host(dir: &tempfile::TempDir) -> (Arc<crate::host::Host>, PseudonymKey) {
    let host = offline_host(dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    super::onboarding_bounds::load_membership(&host);
    let (me, bea) = (set_our_pseudonym(&host), PseudonymKey([7; 32]));
    HostGovernance::new(Arc::clone(&host.ctx))
        .set_governance_state("c1", removing(me, bea.clone(), false));
    (host, bea)
}

pub(super) fn data(answer: rekindle_ipc::IpcResponse) -> serde_json::Value {
    match answer {
        rekindle_ipc::IpcResponse::Ok { data, .. } => data,
        other => panic!("not an answer: {other:?}"),
    }
}

/// R3-5: every community read the frontends render (the window's views,
/// the list, the info) names our removal and its author.
#[tokio::test]
async fn views_list_and_info_carry_our_removal() {
    use crate::daemon::dispatch::community::{handle_info, handle_list};
    use crate::daemon::DaemonState;
    let dir = tempfile::tempdir().unwrap();
    let (host, bea) = removed_host(&dir).await;
    let views = HostGovernance::new(Arc::clone(&host.ctx))
        .community_views()
        .await;
    let removed = views[0]
        .removed
        .clone()
        .expect("the view carries the removal");
    assert_eq!(removed.by, Some(bea.to_hex()));
    assert!(!removed.banned);

    let list = data(handle_list(&host.ctx, DaemonState::Operational));
    assert_eq!(list[0]["removed"]["by"], serde_json::json!(bea.to_hex()));
    let info = data(handle_info(&host.ctx, DaemonState::Operational, "c1").await);
    assert_eq!(info["removed"]["by"], serde_json::json!(bea.to_hex()));
    host.shutdown().await.unwrap();
}

/// R3-5: Forget deletes a removed community's local state; a community we
/// belong to is refused (a member leaves).
#[tokio::test]
async fn forget_deletes_only_a_community_we_were_removed_from() {
    use crate::host::adapters::governance::ForgetError;
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    super::onboarding_bounds::load_membership(&host);
    let governance = HostGovernance::new(Arc::clone(&host.ctx));
    assert!(matches!(
        governance.forget("c1").await,
        Err(ForgetError::NotRemoved)
    ));
    let answer = crate::daemon::dispatch::community::handle_forget(
        &host.ctx,
        crate::daemon::DaemonState::Operational,
        "c1",
    )
    .await;
    assert!(
        matches!(answer, rekindle_ipc::IpcResponse::Error { code: 409, .. }),
        "{answer:?}"
    );

    let me = set_our_pseudonym(&host);
    governance.set_governance_state("c1", removing(me, PseudonymKey([7; 32]), false));
    governance.forget("c1").await.expect("forgotten");
    assert!(host.ctx.communities.map().read().get("c1").is_none());
    assert!(host.ctx.resolve_community("c1").is_err(), "membership gone");
    let db = host.ctx.db.current().unwrap();
    let rows = db
        .call(move |conn| rekindle_db::repo::communities::load_all(conn, &owner))
        .await
        .unwrap();
    assert!(rows.is_empty(), "community row gone");
    host.shutdown().await.unwrap();
}

/// Forget is Destructive: held for the typed community name (ADR 0019).
#[tokio::test]
async fn forget_is_held_by_the_destructive_gate() {
    use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
    use rekindle_ipc::HostApi as _;
    use rekindle_types::intent::{IntentClass, ProofRequired};
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = removed_host(&dir).await;
    let caller = rekindle_ipc::Caller {
        conn: rekindle_ipc::ConnId::Bus(1),
        clearance: rekindle_ipc::message::SecurityLevel::Admin,
        verified_name: None,
        static_key: None,
    };
    let forget = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityForget {
            anchor: "c1".into(),
        },
    ));
    match host.call(caller, forget).await {
        rekindle_ipc::IpcResponse::IntentRequired(review) => {
            assert_eq!(review.class, IntentClass::Destructive);
            assert_eq!(review.proof, ProofRequired::TypeSubject);
        }
        other => panic!("expected a review, got {other:?}"),
    }
    assert!(
        host.ctx.communities.map().read().get("c1").is_some(),
        "nothing is forgotten before the commit"
    );
    host.shutdown().await.unwrap();
}

/// R3-4: a removed member's refused send names what it can do (a new
/// invite, or Forget), never the CLI's bare-403 "unlock".
#[tokio::test]
async fn a_removed_send_is_refused_with_the_removal_remediation() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = removed_host(&dir).await;
    let channel = "0c".repeat(16);
    let refused = HostChannel::new(&host.ctx)
        .send("c1", &channel, "hello", None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            refused,
            rekindle_channel::error::ChannelError::Removed { banned: false }
        ),
        "{refused:?}"
    );
    match crate::host::adapters::channel::refusal(&refused) {
        rekindle_ipc::IpcResponse::Error {
            code, remediation, ..
        } => {
            assert_eq!(code, 403);
            let hint = remediation.expect("a removal names what to do");
            assert!(
                hint.contains("new invite") && !hint.contains("unlock"),
                "{hint}"
            );
        }
        other => panic!("not a refusal: {other:?}"),
    }
    host.shutdown().await.unwrap();
}
