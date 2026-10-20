use std::sync::atomic::{AtomicUsize, Ordering};

use zeroize::Zeroizing;

use super::*;

/// A connection of an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Who {
    conn: u8,
    identity: &'static str,
}

impl GateOwner for Who {
    type Identity = &'static str;
    fn identity(&self) -> &&'static str {
        &self.identity
    }

    fn may_act_for(&self, held: &Self) -> bool {
        self == held
    }
}

const ME: Who = Who {
    conn: 1,
    identity: "me",
};

fn who(conn: u8) -> Who {
    Who {
        conn,
        identity: "me",
    }
}

type TestGate = IntentGate<&'static str, Who, &'static str>;

fn gate() -> TestGate {
    IntentGate::new(Arc::new(AttemptLimiter::default()))
}

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

fn held(gate: &TestGate, s: Selection) -> IntentReview {
    match gate.begin(ME, s, "act", 0).unwrap() {
        Gate::Review(r) => r,
        Gate::Run(_) => panic!("expected a review"),
    }
}

fn no_passphrase(_: &str) -> bool {
    panic!("no passphrase expected")
}

#[test]
fn routine_runs_at_once_and_self_proving_kinds_too() {
    let gate = gate();
    let run = gate.begin(
        ME,
        selection(IntentKind::SendChannelMessage, vec![]),
        "send",
        0,
    );
    assert!(matches!(run, Ok(Gate::Run("send"))));
    let login = gate.begin(ME, selection(IntentKind::Login, vec![]), "login", 0);
    assert!(matches!(login, Ok(Gate::Run("login"))));
}

#[test]
fn an_escalated_routine_action_is_held() {
    let gate = gate();
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
    let gate = gate();
    let s = selection(IntentKind::BlockUser, vec![]);
    let r = held(&gate, s.clone());
    let commit = || {
        gate.commit(
            r.intent_id,
            &ME,
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
    let gate = gate();
    let s = selection(IntentKind::LeaveCommunity, vec![]);
    let r = held(&gate, s.clone());
    let fresh = selection(IntentKind::LeaveCommunity, vec![Escalation::LastAdmin]);
    let Ok(Gate::Review(again)) = gate.commit(
        r.intent_id,
        &ME,
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
            r.intent_id,
            &ME,
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
        gate.commit(again.intent_id, &ME, &typed, fresh, no_passphrase, 2),
        Ok(Gate::Run("act"))
    ));
}

#[test]
fn the_proof_must_match_and_a_wrong_one_keeps_it_held() {
    let gate = gate();
    let s = selection(
        IntentKind::DeleteChannel,
        vec![Escalation::ContainerNotEmpty {
            messages: 10,
            members: 5,
        }],
    );
    let r = held(&gate, s.clone());
    let try_with = |p: IntentProof| gate.commit(r.intent_id, &ME, &p, s.clone(), no_passphrase, 1);
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
    let gate = gate();
    let s = selection(IntentKind::DeleteIdentity, vec![]);
    let r = held(&gate, s.clone());
    let pass = IntentProof::Passphrase(Zeroizing::new("hunter2".into()));
    assert_eq!(
        gate.commit(r.intent_id, &ME, &pass, s.clone(), |p| p == "other", 1)
            .unwrap_err(),
        GateError::WrongProof
    );
    assert!(matches!(
        gate.commit(r.intent_id, &ME, &pass, s, |p| p == "hunter2", 1),
        Ok(Gate::Run(_))
    ));
}

#[test]
fn held_actions_belong_to_their_owner_expire_and_are_bounded() {
    let gate = gate();
    let s = selection(IntentKind::RemoveFriend, vec![]);
    let r = held(&gate, s.clone());
    assert_eq!(
        gate.commit(
            r.intent_id,
            &who(2),
            &IntentProof::Commit,
            s.clone(),
            no_passphrase,
            1
        )
        .unwrap_err(),
        GateError::UnknownIntent,
        "another owner cannot commit it"
    );
    assert!(gate.cancel(r.intent_id, &who(2)).is_err());
    assert_eq!(
        gate.commit(
            r.intent_id,
            &ME,
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
        gate.begin(ME, s.clone(), "act", 0).unwrap_err(),
        GateError::TooManyPending
    );
    assert!(gate.begin(who(2), s.clone(), "act", 0).is_ok(), "per owner");
    gate.clear(&ME);
    assert!(gate.begin(ME, s, "act", 0).is_ok());
}

/// Five wrong passphrases lock the identity for a minute, and a second
/// connection of the same identity sees the lock (plan U1.9).
#[test]
fn wrong_passphrases_lock_the_identity_across_connections() {
    let gate = gate();
    let s = selection(IntentKind::DeleteIdentity, vec![]);
    let r = held(&gate, s.clone());
    let wrong = IntentProof::Passphrase(Zeroizing::new("nope".into()));
    for _ in 0..5 {
        assert_eq!(
            gate.commit(r.intent_id, &ME, &wrong, s.clone(), |_| false, 10)
                .unwrap_err(),
            GateError::WrongProof
        );
    }
    let second = match gate.begin(who(2), s.clone(), "act", 10).unwrap() {
        Gate::Review(r) => r,
        Gate::Run(_) => panic!("held"),
    };
    let right = IntentProof::Passphrase(Zeroizing::new("hunter2".into()));
    let locked = gate
        .commit(
            second.intent_id,
            &who(2),
            &right,
            s.clone(),
            |_| panic!("a locked identity's passphrase is not checked"),
            20,
        )
        .unwrap_err();
    assert!(
        matches!(locked, GateError::Locked { retry_after_ms } if retry_after_ms == 60_000 - 10)
    );
    assert!(matches!(
        gate.commit(
            second.intent_id,
            &who(2),
            &right,
            s,
            |p| p == "hunter2",
            60_010
        ),
        Ok(Gate::Run("act"))
    ));
}

/// Two commits of one id run the action once and both get its result,
/// concurrently or one after the other (plan U1.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_double_commit_runs_once_and_both_see_the_result() {
    let gate = Arc::new(gate());
    let s = selection(IntentKind::BlockUser, vec![]);
    let r = held(&gate, s.clone());
    let runs = Arc::new(AtomicUsize::new(0));
    let commit = |gate: Arc<TestGate>, runs: Arc<AtomicUsize>, s: Selection| async move {
        gate.commit_and_run(
            r.intent_id,
            &ME,
            &IntentProof::Commit,
            s,
            no_passphrase,
            1,
            |a| {
                let runs = Arc::clone(&runs);
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    a
                }
            },
        )
        .await
    };
    let (a, b) = tokio::join!(
        tokio::spawn(commit(Arc::clone(&gate), Arc::clone(&runs), s.clone())),
        tokio::spawn(commit(Arc::clone(&gate), Arc::clone(&runs), s.clone())),
    );
    for outcome in [a.unwrap(), b.unwrap()] {
        assert!(matches!(outcome, Ok(Committed::Ran("act"))));
    }
    let later = commit(Arc::clone(&gate), Arc::clone(&runs), s).await;
    assert!(matches!(later, Ok(Committed::Ran("act"))));
    assert_eq!(runs.load(Ordering::SeqCst), 1);

    // Another connection presenting the id gets nothing.
    let other = gate
        .commit_and_run(
            r.intent_id,
            &who(2),
            &IntentProof::Commit,
            selection(IntentKind::BlockUser, vec![]),
            no_passphrase,
            2,
            |a| async move { a },
        )
        .await;
    assert_eq!(other.unwrap_err(), GateError::UnknownIntent);
}

/// A refused commit is not remembered: the corrected proof runs.
#[tokio::test]
async fn a_wrong_proof_is_not_cached() {
    let gate = gate();
    let s = selection(
        IntentKind::DeleteChannel,
        vec![Escalation::ContainerNotEmpty {
            messages: 10,
            members: 5,
        }],
    );
    let r = held(&gate, s.clone());
    let run = |a| async move { a };
    let wrong = gate
        .commit_and_run(
            r.intent_id,
            &ME,
            &IntentProof::Typed("x".into()),
            s.clone(),
            no_passphrase,
            1,
            run,
        )
        .await;
    assert_eq!(wrong.unwrap_err(), GateError::WrongProof);
    let right = gate
        .commit_and_run(
            r.intent_id,
            &ME,
            &IntentProof::Typed("general".into()),
            s,
            no_passphrase,
            2,
            run,
        )
        .await;
    assert!(matches!(right, Ok(Committed::Ran("act"))));
}

#[test]
fn the_held_action_is_readable_by_its_owner_only() {
    let gate = gate();
    let review = held(&gate, selection(IntentKind::DeleteChannel, vec![]));
    assert_eq!(gate.held_action(review.intent_id, &ME), Some("act"));
    assert_eq!(gate.held_action(review.intent_id, &who(2)), None);
}

#[test]
fn clear_matching_drops_the_matching_owners_actions_only() {
    let gate = gate();
    let mine = held(&gate, selection(IntentKind::DeleteChannel, vec![]));
    let theirs = match gate
        .begin(
            Who {
                conn: 9,
                identity: "you",
            },
            selection(IntentKind::DeleteChannel, vec![]),
            "act",
            0,
        )
        .unwrap()
    {
        Gate::Review(r) => r,
        Gate::Run(_) => panic!("expected a review"),
    };
    gate.clear_matching(|owner| owner.identity == "me");
    assert!(gate.selection(mine.intent_id, &ME).is_none());
    assert!(gate
        .selection(
            theirs.intent_id,
            &Who {
                conn: 9,
                identity: "you"
            }
        )
        .is_some());
}

#[test]
fn a_backend_hold_is_always_held_and_lives_a_day() {
    let gate = gate();
    // Routine would run at once if a user began it; a hold never does.
    let s = selection(IntentKind::SetFriendNickname, vec![]);
    let review = gate
        .hold(
            ME,
            s.clone(),
            "accept",
            Some("decline"),
            BACKEND_HOLD_TTL_MS,
            0,
        )
        .unwrap();
    assert_eq!(review.expires_at_ms, BACKEND_HOLD_TTL_MS);
    assert!(gate.held_action(review.intent_id, &ME).is_some());
    // Past the five-minute user TTL it is still committable.
    let ran = gate
        .commit(
            review.intent_id,
            &ME,
            &IntentProof::Commit,
            s,
            no_passphrase,
            INTENT_TTL_MS + 1,
        )
        .unwrap();
    assert!(matches!(ran, Gate::Run("accept")));
}

#[test]
fn cancelling_a_hold_hands_back_its_cancel_action() {
    let gate = gate();
    let s = selection(IntentKind::DeleteChannel, vec![]);
    let hold = gate
        .hold(
            ME,
            s.clone(),
            "accept",
            Some("decline"),
            BACKEND_HOLD_TTL_MS,
            0,
        )
        .unwrap();
    // Its Cancel sends something, so only an explicit Cancel may cancel it.
    assert!(hold.cancel_sends);
    assert_eq!(
        gate.cancel(hold.intent_id, &who(2)),
        Err(GateError::UnknownIntent),
        "not theirs"
    );
    assert_eq!(gate.cancel(hold.intent_id, &ME), Ok(Some("decline")));
    assert_eq!(
        gate.cancel(hold.intent_id, &ME),
        Err(GateError::UnknownIntent),
        "gone"
    );
    let plain = held(&gate, s);
    assert!(!plain.cancel_sends);
    assert_eq!(gate.cancel(plain.intent_id, &ME), Ok(None));
}

#[test]
fn retain_held_drops_what_it_rejects_without_its_cancel_action() {
    let gate = gate();
    let s = selection(IntentKind::DeleteChannel, vec![]);
    let old = gate
        .hold(
            ME,
            s.clone(),
            "old",
            Some("decline-old"),
            BACKEND_HOLD_TTL_MS,
            0,
        )
        .unwrap();
    let kept = held(&gate, s);
    gate.retain_held(|_, action| *action != "old");
    assert!(gate.held_action(old.intent_id, &ME).is_none());
    assert_eq!(gate.held_action(kept.intent_id, &ME), Some("act"));
}
