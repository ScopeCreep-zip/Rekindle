//! Invite expiry is a Host timer on the Host clock (plan E3.1 C2b): an
//! invite made to last two minutes is listed until then, expires at its
//! time on the simulated clock (`InviteExpired` from the inviter's Host),
//! and is then refused to a joiner through the inviter's `InviteRevoked`
//! (register SEC-3) once that revocation is on the inviter's frontier: the
//! joiner's merged state no longer holds the invite. Before then the
//! joiner refuses it on its own clock as expired.

use std::time::Duration;

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Refused};

const TTL: u64 = 120;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_invite_expires_at_its_time_and_is_then_refused() {
    let community = Community::create(2, 211).await;
    let events = community.nodes[0].events().await;
    // Before the request: the invite's expiry is minted inside it.
    let asked = tokio::time::Instant::now();
    let invite = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteCreate {
                community: community.anchor.clone(),
                max_uses: 1,
                expires_seconds: Some(TTL),
            },
        )))
        .await;
    let link = invite["url"].as_str().expect("an invite link").to_string();
    let listed = |v: &serde_json::Value| v.as_array().is_some_and(|a| !a.is_empty());
    let list = IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::InviteList {
        community: community.anchor.clone(),
    }));
    assert!(
        listed(&community.nodes[0].ok(list.clone()).await),
        "listed while live"
    );

    let expired = eventually(&community.sim, Duration::from_secs(TTL * 2), || async {
        events.any(|e| {
            matches!(
                e,
                SubscriptionEvent::Governance(GovernanceEvent::InviteExpired { community: c, .. })
                    if *c == community.anchor
            )
        })
    })
    .await;
    assert!(expired, "the inviter's Host expires the invite");
    let at = asked.elapsed();
    assert!(
        at >= Duration::from_secs(TTL) && at < Duration::from_secs(TTL + 30),
        "at its time: {at:?}"
    );
    assert!(
        !listed(&community.nodes[0].ok(list).await),
        "no longer listed"
    );

    // `InviteExpired` is journaled at the invite's time, before the
    // inviter's `InviteRevoked` lands on its op log and frontier (`M`/2,
    // `M`/3). A join meanwhile walks a frontier without the revocation:
    // its merged state still holds the invite and the joiner refuses it on
    // its own clock as expired. Once the revocation is on the frontier,
    // the walk reaches it and the join is refused as revoked. Either way
    // the expired invite is never admitted, and the joiner is told the one
    // typed reason, 410 `INVITE_NO_LONGER_VALID` (`join_flow::check_invite`;
    // `rekindle_types::join_refusal`: a revocation is not told apart from
    // the clock).
    for phase in ["before the revocation lands", "after it is on the frontier"] {
        let joined = community.nodes[1]
            .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityJoin {
                    invite: link.clone(),
                },
            )))
            .await;
        assert!(
            matches!(&joined, Err(Refused { code: 410, message, .. })
                if message == "This invite is no longer valid."),
            "an expired invite is never admitted ({phase}): {joined:?}"
        );
        tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    }
}
