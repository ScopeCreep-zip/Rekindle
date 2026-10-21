//! Intent review shapes (plan U1.9): one sample per variant of every
//! serialized intent type, pinned in the same golden file as the events
//! they ride in (`IntentPending`, D3). Each enum has an exhaustive name
//! match, so a new variant does not compile until it is named, and the
//! coverage test fails until it has a sample.

use rekindle_types::intent::{
    AttachmentFacts, Escalation, Fact, IntentClass, IntentId, IntentKind, IntentProof,
    IntentReview, Notifies, ProofRequired, Reversibility, Subject,
};
use serde::Serialize;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::{pk, rk};

pub fn id() -> IntentId {
    "01900000-0000-7000-8000-000000000001"
        .parse()
        .expect("uuid")
}

fn class_name(c: IntentClass) -> &'static str {
    match c {
        IntentClass::Routine => "routine",
        IntentClass::Consequential => "consequential",
        IntentClass::TrustChanging => "trustChanging",
        IntentClass::Destructive => "destructive",
        IntentClass::Protected => "protected",
    }
}

fn proof_required_name(p: ProofRequired) -> &'static str {
    match p {
        ProofRequired::None => "none",
        ProofRequired::Commit => "commit",
        ProofRequired::Acknowledge => "acknowledge",
        ProofRequired::TypeSubject => "typeSubject",
        ProofRequired::Passphrase => "passphrase",
    }
}

fn escalation_name(e: &Escalation) -> &'static str {
    match e {
        Escalation::VerifiedKeyChanged { .. } => "verifiedKeyChanged",
        Escalation::MassMention { .. } => "massMention",
        Escalation::Attachment(_) => "attachment",
        Escalation::CrossAudience { .. } => "crossAudience",
        Escalation::PendingWork { .. } => "pendingWork",
        Escalation::LastAdmin => "lastAdmin",
        Escalation::SoleMember => "soleMember",
        Escalation::ContainerNotEmpty { .. } => "containerNotEmpty",
        Escalation::StartsMekBallot => "startsMekBallot",
        Escalation::FirstMediaInCall => "firstMediaInCall",
    }
}

fn fact_name(f: &Fact) -> &'static str {
    match f {
        Fact::SubjectNotTold => "subjectNotTold",
        Fact::PendingMessagesDropped => "pendingMessagesDropped",
        Fact::SessionDeleted => "sessionDeleted",
        Fact::SessionKeptUntilReplaced => "sessionKeptUntilReplaced",
        Fact::UnopenedMessagesLost => "unopenedMessagesLost",
        Fact::Fingerprint { .. } => "fingerprint",
        Fact::SafetyNumber { .. } => "safetyNumber",
        Fact::SafetyNumberChanged { .. } => "safetyNumberChanged",
        Fact::MembersAffected { .. } => "membersAffected",
        Fact::MessagesDeleted { .. } => "messagesDeleted",
        Fact::PresenceVisible => "presenceVisible",
        Fact::IpAddressLeaves => "ipAddressLeaves",
        Fact::RejoinNeedsInvite => "rejoinNeedsInvite",
    }
}

fn subject_name(s: &Subject) -> &'static str {
    match s {
        Subject::Person { .. } => "person",
        Subject::Community { .. } => "community",
        Subject::Channel { .. } => "channel",
        Subject::Role { .. } => "role",
        Subject::File { .. } => "file",
        Subject::Link { .. } => "link",
        Subject::OwnIdentity { .. } => "ownIdentity",
        Subject::Named { .. } => "named",
    }
}

fn reversibility_name(r: Reversibility) -> &'static str {
    match r {
        Reversibility::Undoable => "undoable",
        Reversibility::ReversibleByInverse(_) => "reversibleByInverse",
        Reversibility::NeedsOthers => "needsOthers",
        Reversibility::Never => "never",
    }
}

fn notifies_name(n: Notifies) -> &'static str {
    match n {
        Notifies::Nobody => "nobody",
        Notifies::Subject => "subject",
        Notifies::Members(_) => "members",
    }
}

fn proof_name(p: &IntentProof) -> &'static str {
    match p {
        IntentProof::Commit => "commit",
        IntentProof::Acknowledged => "acknowledged",
        IntentProof::Typed(_) => "typed",
        IntentProof::Passphrase(_) => "passphrase",
    }
}

const CLASSES: [IntentClass; 5] = [
    IntentClass::Routine,
    IntentClass::Consequential,
    IntentClass::TrustChanging,
    IntentClass::Destructive,
    IntentClass::Protected,
];

const PROOFS_REQUIRED: [ProofRequired; 5] = [
    ProofRequired::None,
    ProofRequired::Commit,
    ProofRequired::Acknowledge,
    ProofRequired::TypeSubject,
    ProofRequired::Passphrase,
];

fn escalations() -> Vec<Escalation> {
    vec![
        Escalation::VerifiedKeyChanged {
            old_safety_number: "1a2b3c4d".into(),
            new_safety_number: "5e6f7a8b".into(),
        },
        Escalation::MassMention { notified: 214 },
        Escalation::Attachment(AttachmentFacts {
            file_name: "photo.jpg".into(),
            size_bytes: 2048,
            metadata_removed: false,
        }),
        Escalation::CrossAudience {
            from_members: 12,
            to_members: 340,
        },
        Escalation::PendingWork {
            unsent: 3,
            in_call: true,
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

fn facts() -> Vec<Fact> {
    vec![
        Fact::SubjectNotTold,
        Fact::PendingMessagesDropped,
        Fact::SessionDeleted,
        Fact::SessionKeptUntilReplaced,
        Fact::UnopenedMessagesLost,
        Fact::Fingerprint {
            fingerprint: "abcd 0123".into(),
        },
        Fact::SafetyNumber {
            safety_number: "1a2b3c4d".into(),
        },
        Fact::SafetyNumberChanged {
            old_safety_number: "1a2b3c4d".into(),
            new_safety_number: "5e6f7a8b".into(),
        },
        Fact::MembersAffected { count: 41 },
        Fact::MessagesDeleted { count: 1204 },
        Fact::PresenceVisible,
        Fact::IpAddressLeaves,
        Fact::RejoinNeedsInvite,
    ]
}

fn subjects() -> Vec<Subject> {
    vec![
        Subject::Person {
            local_nickname: Some("Alice".into()),
            display_name: Some("alice_g".into()),
            fingerprint: "abcd 0123".into(),
        },
        Subject::Community {
            id: rk('g'),
            name: "Raiders".into(),
        },
        Subject::Channel {
            community_name: "Raiders".into(),
            id: crate::channel(),
            name: "general".into(),
        },
        Subject::Role {
            id: "role_1".into(),
            name: "officer".into(),
        },
        Subject::File {
            name: "photo.jpg".into(),
        },
        Subject::Link {
            host: "example.org".into(),
        },
        Subject::OwnIdentity { name: "Me".into() },
        Subject::Named { name: pk('b') },
    ]
}

const REVERSIBILITIES: [Reversibility; 4] = [
    Reversibility::Undoable,
    Reversibility::ReversibleByInverse(IntentKind::UnblockUser),
    Reversibility::NeedsOthers,
    Reversibility::Never,
];

const NOTIFIES: [Notifies; 3] = [Notifies::Nobody, Notifies::Subject, Notifies::Members(57)];

fn proofs() -> Vec<IntentProof> {
    vec![
        IntentProof::Commit,
        IntentProof::Acknowledged,
        IntentProof::Typed("general".into()),
        IntentProof::Passphrase(Zeroizing::new("correct horse".into())),
    ]
}

pub fn review() -> IntentReview {
    IntentReview {
        intent_id: id(),
        kind: IntentKind::DeleteChannel,
        class: IntentClass::Destructive,
        subject: subjects().remove(2),
        facts: vec![Fact::MessagesDeleted { count: 1204 }],
        escalations: vec![Escalation::ContainerNotEmpty {
            messages: 1204,
            members: 57,
        }],
        reversibility: Reversibility::Never,
        notifies: Notifies::Members(57),
        proof: ProofRequired::TypeSubject,
        expires_at_ms: 1_700_000_300_000,
        cancel_sends: true,
    }
}

fn json(v: &impl Serialize) -> Value {
    serde_json::to_value(v).expect("serialize intent sample")
}

/// Every intent sample, as `(id, JSON)`. Ids start `intent.<type>.`.
pub fn samples() -> Vec<(String, Value)> {
    let mut out = vec![
        ("intent.intentId".to_owned(), json(&id())),
        (
            "intent.intentKind.deleteChannel".to_owned(),
            json(&IntentKind::DeleteChannel),
        ),
        ("intent.intentReview".to_owned(), json(&review())),
    ];
    out.extend(
        CLASSES
            .iter()
            .map(|c| (format!("intent.intentClass.{}", class_name(*c)), json(c))),
    );
    out.extend(PROOFS_REQUIRED.iter().map(|p| {
        (
            format!("intent.proofRequired.{}", proof_required_name(*p)),
            json(p),
        )
    }));
    out.extend(
        escalations()
            .iter()
            .map(|e| (format!("intent.escalation.{}", escalation_name(e)), json(e))),
    );
    out.extend(
        facts()
            .iter()
            .map(|f| (format!("intent.fact.{}", fact_name(f)), json(f))),
    );
    out.extend(
        subjects()
            .iter()
            .map(|s| (format!("intent.subject.{}", subject_name(s)), json(s))),
    );
    out.extend(REVERSIBILITIES.iter().map(|r| {
        (
            format!("intent.reversibility.{}", reversibility_name(*r)),
            json(r),
        )
    }));
    out.extend(
        NOTIFIES
            .iter()
            .map(|n| (format!("intent.notifies.{}", notifies_name(*n)), json(n))),
    );
    out.extend(
        proofs()
            .iter()
            .map(|p| (format!("intent.intentProof.{}", proof_name(p)), json(p))),
    );
    out
}

/// The variant names each sampled enum must cover.
const EXPECTED: &[(&str, usize)] = &[
    ("intentClass", 5),
    ("proofRequired", 5),
    ("escalation", 10),
    ("fact", 13),
    ("subject", 8),
    ("reversibility", 4),
    ("notifies", 3),
    ("intentProof", 4),
];

#[test]
fn every_intent_variant_has_a_sample() {
    let samples = samples();
    for (ty, count) in EXPECTED {
        let prefix = format!("intent.{ty}.");
        let distinct: std::collections::BTreeSet<&str> = samples
            .iter()
            .filter_map(|(id, _)| id.strip_prefix(&prefix))
            .collect();
        assert_eq!(distinct.len(), *count, "{ty}: one sample per variant");
    }
    let ids: std::collections::BTreeSet<&str> = samples.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids.len(), samples.len(), "duplicate intent sample ids");
}
