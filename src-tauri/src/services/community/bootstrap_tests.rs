//! The desktop's wire answer to a joiner's `BootstrapRequest` (plan E3.2
//! S10 Verify, "a bootstrap build failure answers a NAK, both tracks"): a
//! retry later and a typed refusal reach the joiner as their own bytes,
//! which it reads as no refusal and as the named reason; only a request
//! the Host cannot answer at all is a bare NAK, which withdraws the join
//! (BR1, IJ2, JN1). The runtime's half (`answer_bootstrap_request` over a
//! shell Host's `HostGovernance`) is `rekindle-node`'s
//! `host/bootstrap_unresolved_tests.rs`.

use rekindle_governance_runtime::bootstrap_peers::read_reply;
use rekindle_governance_runtime::{
    encode_refusal, BootstrapAnswer, BootstrapJoiner, GovernanceRuntimeError,
};
use rekindle_types::governance::{OpHash, OpRef};
use rekindle_types::id::PseudonymKey;
use rekindle_types::join_refusal::JoinRefusal;

use super::{judge, wire_reply};

const UNHELD_COMMUNITY: &str = "c1";

#[test]
fn a_retry_later_reaches_the_joiner_as_no_refusal() {
    let reply = wire_reply(
        UNHELD_COMMUNITY,
        Ok(BootstrapAnswer::RetryLater(
            rekindle_types::message::CALL_RETRY_LATER.to_vec(),
        )),
    );
    assert_eq!(reply, rekindle_types::message::CALL_RETRY_LATER);
    assert!(
        matches!(
            read_reply(&reply, UNHELD_COMMUNITY),
            Err(GovernanceRuntimeError::PeerRetryLater)
        ),
        "the joiner retries; its join is not withdrawn"
    );
}

#[test]
fn a_typed_refusal_reaches_the_joiner_with_its_reason() {
    let refusal = encode_refusal(UNHELD_COMMUNITY, JoinRefusal::InviteNoLongerValid).unwrap();
    let reply = wire_reply(UNHELD_COMMUNITY, Ok(BootstrapAnswer::Refused(refusal)));
    let read = read_reply(&reply, UNHELD_COMMUNITY);
    assert!(
        matches!(
            read,
            Err(GovernanceRuntimeError::BootstrapRefused(r)) if r == JoinRefusal::InviteNoLongerValid
        ),
        "the reason is not dropped to a bare NAK, got {read:?}"
    );
}

fn stranger() -> BootstrapJoiner {
    let author = PseudonymKey([0xB7; 32]);
    BootstrapJoiner {
        pseudonym: hex::encode(author.0),
        member_joined: OpRef {
            op_hash: OpHash([0; 32]),
            device_key: author.0,
            author,
            author_record: "VLD0:joiner-record".into(),
            author_seq: 0,
        },
        member_joined_op: Vec::new(),
        route: Vec::new(),
    }
}

/// A request that arrives with no unlocked Host has no answer: the one
/// bare NAK. (A community the Host does not hold is not this case: its
/// unreadable partial flag is held partial and answered `bootstrapPartial`,
/// S7c.8 (i).)
#[tokio::test]
async fn a_request_the_host_cannot_answer_is_a_nak() {
    let state = std::sync::Arc::new(crate::state::AppState::default());
    let answer = judge(&state, UNHELD_COMMUNITY, &stranger()).await;
    assert!(answer.is_err(), "no Host: no answer, got {answer:?}");
    let reply = wire_reply(UNHELD_COMMUNITY, answer);
    assert_eq!(reply, rekindle_transport::operations::direct::NAK);
    assert!(
        matches!(
            read_reply(&reply, UNHELD_COMMUNITY),
            Err(GovernanceRuntimeError::BootstrapRefused(
                JoinRefusal::NotAdmitted
            ))
        ),
        "a bare NAK is the unnamed refusal"
    );
}
