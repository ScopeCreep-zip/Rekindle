//! A community-join invite through the gate (plan E3.1 D2): held for review
//! with the record-key fingerprint as its subject, never run unreviewed,
//! and refused, not held, before an identity is unlocked (the shell keeps
//! an OS link as raw input until the unlock and submits it then).

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::intent::{IntentClass, IntentKind, Subject};

use super::tests::host_with_identity;

const INVITE_RECORD_KEY: &str = "VLD0:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn join() -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::CommunityJoin {
        invite: format!("rekindle://invite/{INVITE_RECORD_KEY}/{INVITE_RECORD_KEY}/3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c"),
    }))
}

/// From the window, as the shell submits an OS link: held as a
/// TrustChanging `JoinCommunity` whose subject is the fingerprint of the
/// invite's community record key.
#[tokio::test]
async fn a_join_link_is_held_with_its_record_key_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let (host, ann) = host_with_identity(&dir).await;
    let unlocked = host
        .call(
            Caller::shell(),
            IpcRequest::Unlock {
                identity: ann,
                passphrase: "pass".into(),
            },
        )
        .await;
    assert!(
        !matches!(unlocked, IpcResponse::Error { .. }),
        "{unlocked:?}"
    );
    let review = match host.call(Caller::shell(), join()).await {
        IpcResponse::IntentRequired(review) => review,
        other => panic!("a join link is held for review, not {other:?}"),
    };
    assert_eq!(review.kind, IntentKind::JoinCommunity);
    assert_eq!(review.class, IntentClass::TrustChanging);
    let fingerprint = rekindle_crypto::identity::record_key_fingerprint(INVITE_RECORD_KEY).unwrap();
    assert_eq!(review.subject, Subject::Named { name: fingerprint });
}

/// Before an unlock no join runs: the gate holds it as for anyone, but its
/// commit is refused, since there is no identity to join as (the shell keeps
/// an OS link as raw input until the unlock and submits it then).
#[tokio::test]
async fn a_join_link_before_unlock_never_runs() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = host_with_identity(&dir).await;
    let answer = host.call(Caller::shell(), join()).await;
    let IpcResponse::IntentRequired(review) = answer else {
        assert!(matches!(answer, IpcResponse::Error { .. }), "{answer:?}");
        return;
    };
    let commit = IpcRequest::IntentCommit {
        intent_id: review.intent_id,
        proof: rekindle_types::intent::IntentProof::Acknowledged,
    };
    let ran = host.call(Caller::shell(), commit).await;
    assert!(matches!(ran, IpcResponse::Error { .. }), "{ran:?}");
}
