use std::collections::HashSet;

use super::*;

fn every_escalation() -> Vec<Escalation> {
    vec![
        Escalation::VerifiedKeyChanged {
            old_safety_number: "1".into(),
            new_safety_number: "2".into(),
        },
        Escalation::MassMention { notified: 214 },
        Escalation::Attachment(AttachmentFacts {
            file_name: "a.jpg".into(),
            size_bytes: 1,
            metadata_removed: false,
        }),
        Escalation::CrossAudience {
            from_members: 12,
            to_members: 340,
        },
        Escalation::PendingWork {
            unsent: 3,
            in_call: false,
            uploading: false,
        },
        Escalation::LastAdmin,
        Escalation::SoleMember,
        Escalation::ContainerNotEmpty {
            messages: 1204,
            members: 57,
        },
        Escalation::StartsMekBallot,
        Escalation::FirstMediaInCall,
    ]
}

#[test]
fn escalations_never_lower_a_class() {
    let all = every_escalation();
    for kind in IntentKind::ALL {
        assert_eq!(effective_class(*kind, &[]), kind.base_class());
        for esc in &all {
            assert!(effective_class(*kind, std::slice::from_ref(esc)) >= kind.base_class());
        }
        assert!(effective_class(*kind, &all) >= kind.base_class());
    }
}

#[test]
fn protective_kinds_never_exceed_consequential() {
    let all = every_escalation();
    for kind in IntentKind::ALL.iter().filter(|k| k.is_protective()) {
        assert_eq!(kind.base_class(), IntentClass::Consequential, "{kind:?}");
        assert_eq!(effective_class(*kind, &all), IntentClass::Consequential);
    }
    assert_eq!(
        effective_class(IntentKind::BlockUser, &[Escalation::LastAdmin]),
        IntentClass::Consequential
    );
}

#[test]
fn escalations_raise_to_their_floor() {
    assert_eq!(
        effective_class(IntentKind::LeaveCommunity, &[Escalation::LastAdmin]),
        IntentClass::Destructive
    );
    assert_eq!(
        effective_class(
            IntentKind::SendChannelMessage,
            &[Escalation::MassMention { notified: 214 }]
        ),
        IntentClass::Consequential
    );
    assert_eq!(
        effective_class(
            IntentKind::StartNativeVideo,
            &[Escalation::FirstMediaInCall]
        ),
        IntentClass::TrustChanging
    );
}

#[test]
fn each_class_demands_its_proof() {
    let subject = Subject::Channel {
        community_name: "c".into(),
        id: "1".into(),
        name: "general".into(),
    };
    assert!(IntentProof::Commit.satisfies(IntentClass::Consequential.proof(), &subject));
    assert!(!IntentProof::Commit.satisfies(IntentClass::TrustChanging.proof(), &subject));
    assert!(IntentProof::Acknowledged.satisfies(IntentClass::TrustChanging.proof(), &subject));
    let typed = |t: &str| IntentProof::Typed(t.into());
    assert!(typed("general").satisfies(IntentClass::Destructive.proof(), &subject));
    assert!(!typed("General").satisfies(IntentClass::Destructive.proof(), &subject));
    assert!(!IntentProof::Commit.satisfies(IntentClass::Destructive.proof(), &subject));
    let pass = IntentProof::Passphrase(Zeroizing::new("p".into()));
    assert!(pass.satisfies(IntentClass::Protected.proof(), &subject));
    assert!(!typed("general").satisfies(IntentClass::Protected.proof(), &subject));
    assert_eq!(format!("{pass:?}"), "Passphrase(<redacted>)");
}

#[test]
fn a_person_is_typed_by_local_nickname_then_fingerprint() {
    let person = |nick: Option<&str>| Subject::Person {
        local_nickname: nick.map(Into::into),
        display_name: Some("Spoofable".into()),
        fingerprint: "ab12".into(),
    };
    assert_eq!(person(Some("Alice")).typed_name(), "Alice");
    assert_eq!(person(None).typed_name(), "ab12");
}

#[test]
fn the_table_names_each_command_once_and_classifies_the_self_proving() {
    let mut seen = HashSet::new();
    for (name, _) in COMMAND_TABLE {
        assert!(seen.insert(*name), "{name} listed twice");
    }
    assert_eq!(
        kind_of("block_user"),
        Some(Classified::Intent(IntentKind::BlockUser))
    );
    assert_eq!(
        kind_of("Ban"),
        Some(Classified::Intent(IntentKind::BanMember))
    );
    assert_eq!(kind_of("get_friends"), Some(Classified::NotAnIntent));
    assert_eq!(kind_of("no_such_command"), None);
    for name in ["login", "create_identity", "Unlock", "IdentityCreate"] {
        let Some(Classified::Intent(kind)) = kind_of(name) else {
            panic!("{name} is an intent");
        };
        assert!(kind.proves_itself());
        assert_eq!(kind.base_class(), IntentClass::Protected);
    }
}

/// The review's wire shape is camelCase throughout (one sample per
/// type; C7.26's rule).
#[test]
fn the_review_serializes_in_camel_case() {
    let review = IntentReview {
        intent_id: "0190".into(),
        kind: IntentKind::DeleteChannel,
        class: IntentClass::Destructive,
        subject: Subject::Channel {
            community_name: "c".into(),
            id: "1".into(),
            name: "general".into(),
        },
        facts: vec![Fact::MessagesDeleted { count: 1204 }],
        escalations: vec![Escalation::ContainerNotEmpty {
            messages: 1204,
            members: 57,
        }],
        reversibility: Reversibility::ReversibleByInverse(IntentKind::CreateChannel),
        notifies: Notifies::Members(57),
        proof: ProofRequired::TypeSubject,
        expires_at_ms: 1,
    };
    let json = serde_json::to_value(&review).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "intentId": "0190",
            "kind": "deleteChannel",
            "class": "destructive",
            "subject": {"type": "channel", "data": {"communityName": "c", "id": "1", "name": "general"}},
            "facts": [{"type": "messagesDeleted", "data": {"count": 1204}}],
            "escalations": [{"type": "containerNotEmpty", "data": {"messages": 1204, "members": 57}}],
            "reversibility": {"type": "reversibleByInverse", "data": "createChannel"},
            "notifies": {"type": "members", "data": 57},
            "proof": "typeSubject",
            "expiresAtMs": 1
        })
    );
    let back: IntentReview = serde_json::from_value(json).unwrap();
    assert_eq!(back, review);
}
