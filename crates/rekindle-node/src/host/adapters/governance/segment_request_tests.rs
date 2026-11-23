//! Plan GI3: a relayed segment-expansion request is answered only by a
//! member holding MANAGE_COMMUNITY under the merged governance.

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_governance::state::MemberState;
use rekindle_types::id::PseudonymKey;

use crate::host::runtime::tests::channel_gap_tests::with_channel;
use crate::host::runtime::tests::{create_ann, offline_host, seed_registry_community};

fn bob() -> PseudonymKey {
    PseudonymKey([9; 32])
}

fn request(community_id: &str) -> ControlPayload {
    ControlPayload::RequestSegmentExpansion {
        community_id: community_id.into(),
        requester_pseudonym: "joiner".into(),
        full_segment_index: 0,
    }
}

/// We are Bob, a plain member of `c1` with no role: the request changes
/// nothing (no expansion is attempted).
#[tokio::test]
async fn a_member_without_manage_community_does_not_expand() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut state = with_channel();
    state.members.insert(
        bob(),
        MemberState {
            member_record: "record-bob".into(),
            member_record_owner: "owner".into(),
            joined: rekindle_governance::fixture::stamp(2),
        },
    );
    (host.ctx.communities).set_governance_state("c1", state.clone());
    if let Some(c1) = host.ctx.communities.map().write().get_mut("c1") {
        c1.my_pseudonym_key = Some(bob().to_hex());
    }
    let governance = host.governance();
    assert!(governance
        .on_segment_expansion_request("c1", &request("c1"))
        .await
        .is_none());
    assert!(governance
        .on_segment_expansion_request("c1", &request("c2"))
        .await
        .is_none());
    assert_eq!(
        host.ctx.communities.governance_state("c1"),
        Some(state),
        "the merged governance is unchanged"
    );
}
