//! The intent types cross the postcard bus (plan D3.3): a commit carries an
//! `IntentProof`, a stream frame can carry an `IntentPending` review, and
//! their adjacently tagged enums must decode on the other side.

use rekindle_types::intent::{
    Escalation, Fact, IntentClass, IntentId, IntentKind, IntentProof, IntentReview, Notifies,
    ProofRequired, Reversibility, Subject,
};
use rekindle_types::subscription_events::{Cursor, Sequenced, StreamFrame, SubscriptionEvent};
use uuid::Uuid;

use super::framing::{decode_frame, encode_frame};
use super::message::{Message, MessageContext, SecurityLevel};
use super::protocol::{BusPayload, IpcRequest};

fn round_trip(payload: BusPayload) -> BusPayload {
    let msg = Message::new(
        &MessageContext::new(Uuid::now_v7()),
        payload,
        SecurityLevel::Open,
        std::time::Instant::now(),
    );
    let bytes = encode_frame(&msg).expect("encodes");
    decode_frame::<Message<BusPayload>>(&bytes)
        .expect("decodes")
        .payload
}

#[test]
fn a_commit_with_a_passphrase_proof_crosses_the_bus() {
    let intent_id = IntentId::v7(1_000, [7; 10]);
    let decoded = round_trip(BusPayload::Request(IpcRequest::IntentCommit {
        intent_id,
        proof: IntentProof::Passphrase("correct horse".to_string().into()),
    }));
    let BusPayload::Request(IpcRequest::IntentCommit {
        intent_id: id,
        proof,
    }) = decoded
    else {
        panic!("not a commit");
    };
    assert_eq!(id, intent_id);
    assert_eq!(
        proof,
        IntentProof::Passphrase("correct horse".to_string().into())
    );
}

#[test]
fn a_pending_review_crosses_the_bus() {
    let review = IntentReview {
        intent_id: IntentId::v7(1_000, [7; 10]),
        kind: IntentKind::LeaveCommunity,
        class: IntentClass::Destructive,
        subject: Subject::Community {
            id: "c".into(),
            name: "Team".into(),
        },
        facts: vec![Fact::RejoinNeedsInvite, Fact::MembersAffected { count: 3 }],
        escalations: vec![Escalation::LastAdmin],
        reversibility: Reversibility::ReversibleByInverse(IntentKind::JoinCommunity),
        notifies: Notifies::Members(3),
        proof: ProofRequired::TypeSubject,
        expires_at_ms: 1,
        cancel_sends: true,
    };
    let decoded = round_trip(BusPayload::Event(StreamFrame::Event(Sequenced {
        cursor: Cursor { epoch: 1, seq: 2 },
        event: SubscriptionEvent::IntentPending(review.clone()),
    })));
    let BusPayload::Event(StreamFrame::Event(Sequenced {
        event: SubscriptionEvent::IntentPending(got),
        ..
    })) = decoded
    else {
        panic!("not a pending review");
    };
    assert_eq!(got, review);
}
