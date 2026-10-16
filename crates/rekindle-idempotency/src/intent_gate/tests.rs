use zeroize::Zeroizing;

use super::*;

fn selection(kind: IntentKind, escalations: Vec<Escalation>) -> Selection {
    Selection {
        kind,
        subject: Subject::Channel {
            community_name: "c".into(),
            id: "1".into(),
            name: "general".into(),
        },
        facts: vec![Fact::MessagesDeleted { count: 10 }],
        escalations,
        reversibility: Reversibility::Never,
        notifies: Notifies::Members(5),
    }
}

fn held(gate: &IntentGate<&'static str, u8>, s: Selection) -> IntentReview {
    match gate.begin(1, s, "act", 0).unwrap() {
        Gate::Review(r) => r,
        Gate::Run(_) => panic!("expected a review"),
    }
}

fn no_passphrase(_: &str) -> bool {
    panic!("no passphrase expected")
}

#[test]
fn routine_runs_at_once_and_self_proving_kinds_too() {
    let gate = IntentGate::<&str, u8>::default();
    let run = gate.begin(
        1,
        selection(IntentKind::SendChannelMessage, vec![]),
        "send",
        0,
    );
    assert!(matches!(run, Ok(Gate::Run("send"))));
    let login = gate.begin(1, selection(IntentKind::Login, vec![]), "login", 0);
    assert!(matches!(login, Ok(Gate::Run("login"))));
}

#[test]
fn an_escalated_routine_action_is_held() {
    let gate = IntentGate::<&str, u8>::default();
    let r = held(
        &gate,
        selection(
            IntentKind::SendChannelMessage,
            vec![Escalation::MassMention { notified: 214 }],
        ),
    );
    assert_eq!(r.class, IntentClass::Consequential);
    assert_eq!(r.proof, ProofRequired::Commit);
}

#[test]
fn a_commit_runs_the_action_once() {
    let gate = IntentGate::<&str, u8>::default();
    let s = selection(IntentKind::BlockUser, vec![]);
    let r = held(&gate, s.clone());
    let commit = || {
        gate.commit(
            &r.intent_id,
            &1,
            &IntentProof::Commit,
            s.clone(),
            no_passphrase,
            1,
        )
    };
    assert!(matches!(commit(), Ok(Gate::Run("act"))));
    assert_eq!(
        commit().unwrap_err(),
        GateError::UnknownIntent,
        "a double click runs once"
    );
}

#[test]
fn a_changed_fact_returns_a_fresh_review_and_runs_nothing() {
    let gate = IntentGate::<&str, u8>::default();
    let s = selection(IntentKind::LeaveCommunity, vec![]);
    let r = held(&gate, s.clone());
    let fresh = selection(IntentKind::LeaveCommunity, vec![Escalation::LastAdmin]);
    let Ok(Gate::Review(again)) = gate.commit(
        &r.intent_id,
        &1,
        &IntentProof::Commit,
        fresh.clone(),
        no_passphrase,
        1,
    ) else {
        panic!("a risen class must be reviewed again");
    };
    assert_eq!(again.class, IntentClass::Destructive);
    assert_ne!(again.intent_id, r.intent_id);
    assert_eq!(
        gate.commit(
            &r.intent_id,
            &1,
            &IntentProof::Commit,
            fresh.clone(),
            no_passphrase,
            1
        )
        .unwrap_err(),
        GateError::UnknownIntent,
        "the old id is gone"
    );
    let typed = IntentProof::Typed("general".into());
    assert!(matches!(
        gate.commit(&again.intent_id, &1, &typed, fresh, no_passphrase, 2),
        Ok(Gate::Run("act"))
    ));
}

#[test]
fn the_proof_must_match_and_a_wrong_one_keeps_it_held() {
    let gate = IntentGate::<&str, u8>::default();
    let s = selection(
        IntentKind::DeleteChannel,
        vec![Escalation::ContainerNotEmpty {
            messages: 10,
            members: 5,
        }],
    );
    let r = held(&gate, s.clone());
    let try_with = |p: IntentProof| gate.commit(&r.intent_id, &1, &p, s.clone(), no_passphrase, 1);
    assert_eq!(
        try_with(IntentProof::Commit).unwrap_err(),
        GateError::WrongProof
    );
    assert_eq!(
        try_with(IntentProof::Typed("generl".into())).unwrap_err(),
        GateError::WrongProof
    );
    assert!(matches!(
        try_with(IntentProof::Typed("general".into())),
        Ok(Gate::Run(_))
    ));
}

#[test]
fn a_passphrase_proof_is_verified_by_the_host() {
    let gate = IntentGate::<&str, u8>::default();
    let s = selection(IntentKind::DeleteIdentity, vec![]);
    let r = held(&gate, s.clone());
    let pass = IntentProof::Passphrase(Zeroizing::new("hunter2".into()));
    assert_eq!(
        gate.commit(&r.intent_id, &1, &pass, s.clone(), |p| p == "other", 1)
            .unwrap_err(),
        GateError::WrongProof
    );
    assert!(matches!(
        gate.commit(&r.intent_id, &1, &pass, s, |p| p == "hunter2", 1),
        Ok(Gate::Run(_))
    ));
}

#[test]
fn held_actions_belong_to_their_owner_expire_and_are_bounded() {
    let gate = IntentGate::<&str, u8>::default();
    let s = selection(IntentKind::RemoveFriend, vec![]);
    let r = held(&gate, s.clone());
    assert_eq!(
        gate.commit(
            &r.intent_id,
            &2,
            &IntentProof::Commit,
            s.clone(),
            no_passphrase,
            1
        )
        .unwrap_err(),
        GateError::UnknownIntent,
        "another owner cannot commit it"
    );
    assert!(!gate.cancel(&r.intent_id, &2));
    assert_eq!(
        gate.commit(
            &r.intent_id,
            &1,
            &IntentProof::Commit,
            s.clone(),
            no_passphrase,
            INTENT_TTL_MS
        )
        .unwrap_err(),
        GateError::UnknownIntent,
        "expired"
    );
    for _ in 1..MAX_PENDING_PER_OWNER {
        held(&gate, s.clone());
    }
    assert_eq!(
        gate.begin(1, s.clone(), "act", 0).unwrap_err(),
        GateError::TooManyPending
    );
    assert!(gate.begin(2, s.clone(), "act", 0).is_ok(), "per owner");
    gate.clear(&1);
    assert!(gate.begin(1, s, "act", 0).is_ok());
}
