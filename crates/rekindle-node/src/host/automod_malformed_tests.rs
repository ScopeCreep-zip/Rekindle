//! A governance entry every reader drops is refused at write (plan E3.1
//! F3h): an automod rule whose regex does not compile is `Malformed`,
//! answered 400, and neither written nor applied, so our gate never
//! diverges from the community's.

use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::governance::AutoModAction;
use rekindle_types::id::PseudonymKey;

use super::onboarding_bounds::load_membership;
use super::{create_ann, offline_host, seed_registry_community};
use crate::daemon::dispatch::context::runtime_refused;
use crate::host::adapters::governance::automod_rules::AutoModRuleEdit;

#[tokio::test]
async fn an_uncompilable_automod_rule_is_refused_with_400_and_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    load_membership(&host);
    let creator = GovernanceState {
        creator: Some(PseudonymKey([0xcd; 32])),
        ..GovernanceState::default()
    };
    {
        let mut communities = host.ctx.communities.map().write();
        let c1 = communities.get_mut("c1").expect("c1 is loaded");
        c1.my_pseudonym_key = Some("cd".repeat(32));
        c1.governance_key = Some("c1".into());
        c1.governance_state = Some(creator.clone());
    }

    let edit = AutoModRuleEdit {
        rule_id: None,
        name: "broken".into(),
        enabled: true,
        keywords: Vec::new(),
        regex_patterns: vec!["(".into()],
        action: AutoModAction::BlockLocally,
    };
    let refused = host
        .governance()
        .set_automod_rule("c1", edit)
        .await
        .expect_err("an uncompilable rule is refused");
    assert!(
        matches!(refused, GovernanceRuntimeError::Malformed),
        "{refused}"
    );
    assert!(matches!(
        runtime_refused(&refused, "automod rule set"),
        IpcResponse::Error { code: 400, .. }
    ));
    let held = host
        .ctx
        .communities
        .map()
        .read()
        .get("c1")
        .and_then(|c| c.governance_state.clone());
    assert_eq!(held, Some(creator), "nothing was applied");
    host.shutdown().await.unwrap();
}
