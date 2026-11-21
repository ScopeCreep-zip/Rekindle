//! An invite's `max_uses` outside `1..=100` is the caller's error (plan
//! E3.2 IJ1): the runtime refuses it as `OutOfBounds`, answered 400 on the
//! daemon's `InviteCreate`, never a 500.

use rekindle_governance_runtime::GovernanceRuntimeError;

use super::onboarding_bounds::load_membership;
use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::governance::invites::InviteRefusal;

#[tokio::test]
async fn an_out_of_range_max_uses_answers_400() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    load_membership(&host);
    let governance = host.governance();

    for max_uses in [0, 101] {
        let refused = governance
            .create_invite("c1", max_uses, None)
            .await
            .unwrap_err();
        assert!(
            matches!(
                refused,
                InviteRefusal::Runtime(GovernanceRuntimeError::OutOfBounds { field: "max_uses" })
            ),
            "{refused}"
        );
        assert_eq!(refused.code(), 400, "max_uses {max_uses}: {refused}");
    }
    host.shutdown().await.unwrap();
}
