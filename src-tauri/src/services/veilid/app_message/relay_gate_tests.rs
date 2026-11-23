//! Plan GI2: the desktop's relay dispatch takes nothing the Host's
//! community gate (`HostGovernance::takes_traffic_from`) does not take,
//! for every arm. Without a Host no relayed Control, watch relay or typing
//! frame reaches local dispatch; the gate's departed-author and
//! own-removal verdicts are the Host's (rekindle-node's
//! `the_host_gate_refuses_a_departed_author_and_all_after_our_removal`).
//! Plan GI3: with a real offline Host on the state, the desktop's relay
//! dispatch drops a departed author's envelope and, once we were removed,
//! everyone's, and admits a member's.

use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, SignedEnvelope};
use rekindle_node::host::test_fixtures::{
    create_ann, mark_departed, note_own_removal, seed_registry_community,
};
use rekindle_types::id::PseudonymKey;

use crate::state::AppState;
use crate::test_support::TestHost;

fn relayed(envelope: &CommunityEnvelope) -> SignedEnvelope {
    relayed_in("community", &PseudonymKey([4; 32]), envelope)
}

fn relayed_in(
    community_id: &str,
    author: &PseudonymKey,
    envelope: &CommunityEnvelope,
) -> SignedEnvelope {
    SignedEnvelope {
        community_id: community_id.into(),
        sender_pseudonym: author.to_hex(),
        message_id: [1; rekindle_codec::envelope::MESSAGE_ID_LEN],
        envelope_bytes: encode_community_envelope(envelope).expect("encodes"),
        signature: Vec::new(),
        ttl: 0,
    }
}

fn kick() -> CommunityEnvelope {
    CommunityEnvelope::control(ControlPayload::Kick {
        target_pseudonym: hex::encode([5u8; 32]),
    })
}

/// A Host holding Ann's community `c1`, running on the state.
async fn host_with_community() -> TestHost {
    let test = TestHost::start().await;
    let owner = create_ann(&test.host).await.public_key;
    seed_registry_community(&test.host, &owner).await;
    test
}

#[tokio::test]
async fn a_departed_authors_relay_is_dropped_and_a_members_admitted() {
    let test = host_with_community().await;
    let (member, departed) = (PseudonymKey([3; 32]), PseudonymKey([4; 32]));
    mark_departed(&test.host, "c1", &departed);
    assert!(
        super::admitted_relay(&test.state, &relayed_in("c1", &member, &kick())).is_some(),
        "a member's relayed Control reaches dispatch"
    );
    assert!(
        super::admitted_relay(&test.state, &relayed_in("c1", &departed, &kick())).is_none(),
        "a departed author's relayed Control is dropped"
    );
    test.shutdown().await;
}

#[tokio::test]
async fn nothing_relayed_is_admitted_after_our_own_removal() {
    let test = host_with_community().await;
    let member = PseudonymKey([3; 32]);
    note_own_removal(&test.host, "c1");
    assert!(
        super::admitted_relay(&test.state, &relayed_in("c1", &member, &kick())).is_none(),
        "a member's relayed Control after our removal is dropped"
    );
    test.shutdown().await;
}

#[test]
fn no_relayed_arm_is_taken_before_the_host_judges_its_author() {
    let state = AppState::default();
    let arms = [
        CommunityEnvelope::control(ControlPayload::Kick {
            target_pseudonym: hex::encode([5u8; 32]),
        }),
        CommunityEnvelope::WatchRelay {
            record_key: "VLD0:record".into(),
            subkey: 1,
            content_hash: "hash".into(),
            observer_pseudonym: hex::encode([4u8; 32]),
        },
    ];
    for envelope in &arms {
        assert!(
            super::admitted_relay(&state, &relayed(envelope)).is_none(),
            "{} reached dispatch with no Host gate",
            super::envelope_kind(envelope)
        );
    }
}
