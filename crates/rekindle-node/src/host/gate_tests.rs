//! The one intent gate, through `HostApi::call` on an offline host
//! (plan D3.3).

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::intent::{IntentClass, IntentProof, IntentReview, ProofRequired};

use crate::host::{claim, identity, Host, HostConfig, HostStores, TransportSource};
use rekindle_ipc::{Caller, ConnId, HostApi};

fn temp_root(dir: &tempfile::TempDir) -> rekindle_db::paths::DataRoot {
    let data = dir.path().join("data");
    rekindle_db::paths::DataRoot {
        config: dir.path().join("config"),
        logs: dir.path().join("logs"),
        state: data.join("state"),
        data,
    }
}

fn bus(conn: u64) -> Caller {
    Caller {
        conn: ConnId::Bus(conn),
        clearance: SecurityLevel::Admin,
        verified_name: None,
        static_key: None,
    }
}

/// A host with one identity, created and locked again.
async fn host_with_identity(dir: &tempfile::TempDir) -> (std::sync::Arc<Host>, String) {
    let host = Host::start(HostConfig {
        root: claim(temp_root(dir)).unwrap(),
        stores: HostStores::default(),
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: std::sync::Arc::new(rekindle_utils::SystemClock),
        rng: std::sync::Arc::new(rekindle_utils::OsRandom),
        shell: None,
    })
    .await
    .unwrap();
    let created = identity::create(host.root(), &host.identity_stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    let public_key = created.public_key.clone();
    drop(created);
    identity::lock(&host.identity_stores()).await;
    (host, public_key)
}

fn review(answer: IpcResponse) -> IntentReview {
    match answer {
        IpcResponse::IntentRequired(review) => review,
        other => panic!("expected a review, got {other:?}"),
    }
}

fn code(answer: &IpcResponse) -> Option<u32> {
    match answer {
        IpcResponse::Error { code, .. } => Some(*code),
        _ => None,
    }
}

/// Deleting an identity is Protected: held for its passphrase, refused on
/// a wrong one (still held), run once on the right one, its answer
/// repeated for the same id, and unknown to another connection.
#[tokio::test]
async fn a_protected_delete_runs_once_on_its_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let (host, public_key) = host_with_identity(&dir).await;
    let delete = IpcRequest::IdentityDelete {
        identity: public_key.clone(),
    };
    let held = review(host.call(bus(1), delete).await);
    assert_eq!(held.class, IntentClass::Protected);
    assert_eq!(held.proof, ProofRequired::Passphrase);

    let commit = |passphrase: &str| IpcRequest::IntentCommit {
        intent_id: held.intent_id,
        proof: IntentProof::Passphrase(passphrase.to_string().into()),
    };
    assert_eq!(code(&host.call(bus(1), commit("wrong")).await), Some(403));
    assert_eq!(code(&host.call(bus(2), commit("pass")).await), Some(404));
    let ran = host.call(bus(1), commit("pass")).await;
    assert!(
        matches!(
            ran,
            IpcResponse::Ok {
                reversibility: Some(_),
                ..
            }
        ),
        "{ran:?}"
    );
    assert!(identity::list(host.root()).unwrap().is_empty());
    let again = host.call(bus(1), commit("pass")).await;
    assert!(matches!(again, IpcResponse::Ok { .. }), "{again:?}");
}

/// A cancelled action cannot be committed; a closed connection's actions
/// are dropped.
#[tokio::test]
async fn cancelled_and_disconnected_actions_are_gone() {
    let dir = tempfile::tempdir().unwrap();
    let (host, public_key) = host_with_identity(&dir).await;
    let delete = || IpcRequest::IdentityDelete {
        identity: public_key.clone(),
    };
    let first = review(host.call(bus(1), delete()).await);
    let cancelled = host
        .call(
            bus(1),
            IpcRequest::IntentCancel {
                intent_id: first.intent_id,
            },
        )
        .await;
    assert!(matches!(cancelled, IpcResponse::Ok { .. }));
    let commit = |id| IpcRequest::IntentCommit {
        intent_id: id,
        proof: IntentProof::Passphrase("pass".to_string().into()),
    };
    assert_eq!(
        code(&host.call(bus(1), commit(first.intent_id)).await),
        Some(404)
    );

    let second = review(host.call(bus(3), delete()).await);
    crate::host::gate::clear_connection(&host.ctx, 3);
    assert_eq!(
        code(&host.call(bus(3), commit(second.intent_id)).await),
        Some(404)
    );
    assert_eq!(identity::list(host.root()).unwrap().len(), 1);
}

/// Ann, unlocked from the window, and a friend who rotated from a key she verified to
/// `rotated` (their `signal_identities` row, as `IdentityRotated` leaves it).
async fn ann_with_rotated_verified_friend(
    dir: &tempfile::TempDir,
) -> (std::sync::Arc<Host>, String) {
    let (host, ann) = host_with_identity(dir).await;
    let unlocked = host
        .call(
            Caller::shell(),
            IpcRequest::Unlock {
                identity: ann.clone(),
                passphrase: "pass".into(),
            },
        )
        .await;
    assert!(code(&unlocked).is_none(), "{unlocked:?}");
    let owner: [u8; 32] = hex::decode(&ann).unwrap().try_into().unwrap();
    let (first, rotated) = (
        rekindle_crypto::Identity::generate().public_key_bytes(),
        rekindle_crypto::Identity::generate().public_key_bytes(),
    );
    let earlier = rekindle_crypto::Identity::generate().public_key_bytes();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            use rekindle_db::repo::signal_identities as ids;
            ids::rotate(conn, &owner, &earlier, &first, 1)?;
            ids::mark_verified(conn, &owner, &first, 2)?;
            ids::rotate(conn, &owner, &first, &rotated, 3)
        })
        .await
        .unwrap();
    (host, hex::encode(rotated))
}

async fn safety_number(host: &Host, peer: &str) -> rekindle_types::dto::friends::SafetyNumberView {
    let answer = host
        .call(
            bus(1),
            IpcRequest::IdentitySafetyNumber { peer: peer.into() },
        )
        .await;
    match answer {
        IpcResponse::Ok { data, .. } => serde_json::from_value(data).unwrap(),
        other => panic!("{other:?}"),
    }
}

/// A send to a friend whose verified key changed is held on
/// `VerifiedKeyChanged` (TrustChanging, plan E2.3); committing it is the
/// acknowledgement, which the send's own row carries; `verified` stays
/// cleared, and marking the new key verified is its own reviewed action.
#[tokio::test]
async fn a_verified_key_change_holds_the_send_until_it_is_acknowledged() {
    use rekindle_types::intent::{Escalation, Fact, IntentKind, Reversibility};
    let dir = tempfile::tempdir().unwrap();
    let (host, peer) = ann_with_rotated_verified_friend(&dir).await;
    let before = safety_number(&host, &peer).await;
    assert!(before.changed && !before.verified);
    assert_eq!(before.digits.len(), 60);
    assert!(before.qr_rows.len() >= 21);

    let send = IpcRequest::DmSend {
        peer_key: peer.clone(),
        body: "still you?".into(),
        reply_to: None,
    };
    let held = review(host.call(bus(1), send).await);
    assert_eq!(held.class, IntentClass::TrustChanging);
    assert!(matches!(
        held.escalations.as_slice(),
        [Escalation::VerifiedKeyChanged { new_safety_number, .. }] if *new_safety_number == before.digits
    ));
    // No session exists offline, so the send itself fails; its row, and
    // the acknowledgement with it, are kept.
    host.call(
        bus(1),
        IpcRequest::IntentCommit {
            intent_id: held.intent_id,
            proof: IntentProof::Acknowledged,
        },
    )
    .await;
    let after = safety_number(&host, &peer).await;
    assert!(!after.changed && !after.verified);

    let mark = review(
        host.call(
            bus(1),
            IpcRequest::IdentityMarkVerified { peer: peer.clone() },
        )
        .await,
    );
    assert_eq!(
        (mark.kind, mark.class),
        (IntentKind::MarkVerified, IntentClass::TrustChanging)
    );
    assert_eq!(mark.proof, ProofRequired::Acknowledge);
    assert_eq!(
        mark.facts,
        [Fact::SafetyNumber {
            safety_number: before.digits.clone()
        }]
    );
    assert_eq!(
        mark.reversibility,
        Reversibility::ReversibleByInverse(IntentKind::ClearVerification)
    );
    let commit = |id| IpcRequest::IntentCommit {
        intent_id: id,
        proof: IntentProof::Acknowledged,
    };
    let marked = host.call(bus(1), commit(mark.intent_id)).await;
    assert!(code(&marked).is_none(), "{marked:?}");
    assert!(safety_number(&host, &peer).await.verified);

    let clear = review(
        host.call(
            bus(1),
            IpcRequest::IdentityClearVerification { peer: peer.clone() },
        )
        .await,
    );
    assert_eq!(clear.kind, IntentKind::ClearVerification);
    let cleared = host.call(bus(1), commit(clear.intent_id)).await;
    assert!(code(&cleared).is_none(), "{cleared:?}");
    assert!(!safety_number(&host, &peer).await.verified);
}

/// Answer `answer`, committing its review when it is one.
async fn settle(host: &Host, answer: IpcResponse) -> IpcResponse {
    use rekindle_types::intent::{IntentProof, ProofRequired};
    let IpcResponse::IntentRequired(held) = answer else {
        return answer;
    };
    let proof = match held.proof {
        ProofRequired::Acknowledge => IntentProof::Acknowledged,
        _ => IntentProof::Commit,
    };
    host.call(
        bus(1),
        IpcRequest::IntentCommit {
            intent_id: held.intent_id,
            proof,
        },
    )
    .await
}

async fn dm_rows(host: &Host, peer: &str) -> Vec<rekindle_types::dto::dm::DmMessage> {
    let answer = host
        .call(
            bus(1),
            IpcRequest::DmHistory {
                peer_key: peer.into(),
                limit: 50,
                before: None,
            },
        )
        .await;
    match answer {
        IpcResponse::Ok { data, .. } => serde_json::from_value(data).unwrap(),
        other => panic!("{other:?}"),
    }
}

/// A manual Retry re-sends the stored row by its client id (U-3): no new
/// row is stored, and an id we never sent is not found.
#[tokio::test]
async fn a_resend_sends_the_stored_row_and_stores_none() {
    use rekindle_types::message::SendState;
    let dir = tempfile::tempdir().unwrap();
    let (host, peer) = ann_with_rotated_verified_friend(&dir).await;
    let send = IpcRequest::DmSend {
        peer_key: peer.clone(),
        body: "once".into(),
        reply_to: None,
    };
    // Offline, the send fails and its row reads failed.
    assert!(code(&settle(&host, host.call(bus(1), send).await).await).is_some());
    let rows = dm_rows(&host, &peer).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].send_state, SendState::Failed);
    let message_id = rows[0].message_id.clone().unwrap();

    let resend = IpcRequest::DmResend {
        message_id: message_id.clone(),
    };
    let answer = settle(&host, host.call(bus(1), resend).await).await;
    assert!(code(&answer).is_some(), "{answer:?}");
    let after = dm_rows(&host, &peer).await;
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].message_id.as_deref(), Some(message_id.as_str()));
    assert_eq!(after[0].send_state, SendState::Failed);

    let unknown = IpcRequest::DmResend {
        message_id: uuid::Uuid::nil().to_string(),
    };
    let answer = settle(&host, host.call(bus(1), unknown).await).await;
    assert!(code(&answer).is_some(), "{answer:?}");
}
