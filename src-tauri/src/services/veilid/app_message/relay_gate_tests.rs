//! Plan GI2: the desktop's relay dispatch takes nothing the Host's
//! community gate (`HostGovernance::takes_traffic_from`) does not take,
//! for every arm. Without a Host no relayed Control, watch relay or typing
//! frame reaches local dispatch; the gate's departed-author and
//! own-removal verdicts are the Host's (rekindle-node's
//! `the_host_gate_refuses_a_departed_author_and_all_after_our_removal`).

use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, SignedEnvelope};

use crate::state::AppState;

fn relayed(envelope: &CommunityEnvelope) -> SignedEnvelope {
    SignedEnvelope {
        community_id: "community".into(),
        sender_pseudonym: hex::encode([4u8; 32]),
        message_id: [1; rekindle_codec::envelope::MESSAGE_ID_LEN],
        envelope_bytes: encode_community_envelope(envelope).expect("encodes"),
        signature: Vec::new(),
        ttl: 0,
    }
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
