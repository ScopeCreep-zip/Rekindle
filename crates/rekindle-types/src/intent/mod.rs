//! Intentional actions: Select → Review → Commit (plan U1, ADR 0019;
//! design `realignment/r9-intents.md` §7).
//!
//! Every user action has a class ([`IntentKind::base_class`]); facts the
//! backend computes when the action is selected ([`Escalation`]) can only
//! raise it ([`effective_class`]). A Routine action runs at once; any other
//! is held by the host's gate and answered with an [`IntentReview`], which
//! every frontend renders the same way, and runs only on a commit carrying
//! the proof its class demands ([`IntentClass::proof`]).
//!
//! Pure data: the gate lives in `rekindle-idempotency::intent_gate`, the
//! facts in the tier crates that own them.
//!
//! Every enum here is externally tagged (`camelCase`), like
//! `SubscriptionEvent`: a commit and an `IntentPending` review cross the
//! postcard bus, which cannot decode an internally or adjacently tagged
//! enum, and the webview reads the same JSON.

mod all;
mod kind;
mod predicates;
mod table;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub use kind::IntentKind;
pub use table::{kind_of, Classified, COMMAND_TABLE};

/// A held action's id: UUID v7, minted by the gate, and the key a repeated
/// commit of it is answered from (plan U1.9). Serializes as the UUID string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IntentId(pub uuid::Uuid);

impl IntentId {
    /// A fresh time-ordered id.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}

impl Default for IntentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for IntentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for IntentId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        uuid::Uuid::parse_str(s).map(Self)
    }
}

/// How deliberate an action must be, in increasing order (r9 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntentClass {
    /// Frequent and reversible: runs at once, with an undo where an inverse
    /// exists.
    Routine,
    /// Others see it or are affected, or the network side cannot be undone.
    Consequential,
    /// Raises exposure or changes which keys are trusted.
    TrustChanging,
    /// Irreversible loss for others or of shared history.
    Destructive,
    /// Root identity or key material.
    Protected,
}

/// The proof a commit must carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProofRequired {
    None,
    /// A verb + object button, no default.
    Commit,
    /// A checkbox stating the salient trust fact, then commit.
    Acknowledge,
    /// Type the subject's name.
    TypeSubject,
    /// Re-enter the passphrase, every time.
    Passphrase,
}

impl IntentClass {
    /// The proof this class demands (r9 §7.2).
    #[must_use]
    pub const fn proof(self) -> ProofRequired {
        match self {
            Self::Routine => ProofRequired::None,
            Self::Consequential => ProofRequired::Commit,
            Self::TrustChanging => ProofRequired::Acknowledge,
            Self::Destructive => ProofRequired::TypeSubject,
            Self::Protected => ProofRequired::Passphrase,
        }
    }
}

/// What an attachment carries out of the app (escalation E3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentFacts {
    pub file_name: String,
    pub size_bytes: u64,
    /// Whether embedded metadata (EXIF/GPS, document author) is removed
    /// before it leaves. False until E5.1 strips it.
    pub metadata_removed: bool,
}

/// A backend fact computed at Select that raises the class (r9 §7.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Escalation {
    /// E1: the peer's identity key changed since it was marked verified.
    #[serde(rename_all = "camelCase")]
    VerifiedKeyChanged {
        old_safety_number: String,
        new_safety_number: String,
    },
    /// E2: a mention would notify more people than are online.
    #[serde(rename_all = "camelCase")]
    MassMention { notified: u32 },
    /// E3: an attachment leaves the app.
    Attachment(AttachmentFacts),
    /// E4: content moves to a different membership.
    #[serde(rename_all = "camelCase")]
    CrossAudience { from_members: u32, to_members: u32 },
    /// E5: undelivered work, an active call or an upload at logout or lock.
    #[serde(rename_all = "camelCase")]
    PendingWork {
        unsent: u32,
        in_call: bool,
        uploading: bool,
    },
    /// E6: we are the community's last admin.
    LastAdmin,
    /// E6: we are the community's only member.
    SoleMember,
    /// E7: the container holds history or members.
    #[serde(rename_all = "camelCase")]
    ContainerNotEmpty { messages: u32, members: u32 },
    /// E8: the action starts a MEK rotation ballot (D20).
    StartsMekBallot,
    /// E9: the first camera or screen share in this call.
    FirstMediaInCall,
}

impl Escalation {
    /// The lowest class this escalation holds an action at.
    #[must_use]
    pub const fn floor(&self) -> IntentClass {
        match self {
            Self::VerifiedKeyChanged { .. } | Self::Attachment(_) | Self::FirstMediaInCall => {
                IntentClass::TrustChanging
            }
            Self::MassMention { .. }
            | Self::CrossAudience { .. }
            | Self::PendingWork { .. }
            | Self::StartsMekBallot => IntentClass::Consequential,
            Self::LastAdmin | Self::SoleMember | Self::ContainerNotEmpty { .. } => {
                IntentClass::Destructive
            }
        }
    }
}

/// The class an action is held at: its base class raised to every
/// escalation's floor, never above Consequential for a protective kind.
#[must_use]
pub fn effective_class(kind: IntentKind, escalations: &[Escalation]) -> IntentClass {
    let raised = escalations
        .iter()
        .map(Escalation::floor)
        .fold(kind.base_class(), Ord::max);
    if kind.is_protective() {
        raised
            .min(IntentClass::Consequential)
            .max(kind.base_class())
    } else {
        raised
    }
}

/// Who or what the action is about. A person leads with our local
/// nickname and its key fingerprint; a peer-chosen display name is never
/// shown alone (it is spoofable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Subject {
    #[serde(rename_all = "camelCase")]
    Person {
        local_nickname: Option<String>,
        display_name: Option<String>,
        fingerprint: String,
    },
    #[serde(rename_all = "camelCase")]
    Community { id: String, name: String },
    #[serde(rename_all = "camelCase")]
    Channel {
        community_name: String,
        id: String,
        name: String,
    },
    #[serde(rename_all = "camelCase")]
    Role { id: String, name: String },
    #[serde(rename_all = "camelCase")]
    File { name: String },
    #[serde(rename_all = "camelCase")]
    Link { host: String },
    /// Our own identity.
    #[serde(rename_all = "camelCase")]
    OwnIdentity { name: String },
    /// Anything else, named.
    #[serde(rename_all = "camelCase")]
    Named { name: String },
}

impl Subject {
    /// What a TypeSubject proof must match: the object's own name, never a
    /// fixed word (r9 §7.5).
    #[must_use]
    pub fn typed_name(&self) -> &str {
        match self {
            Self::Person {
                local_nickname: Some(name),
                ..
            }
            | Self::Community { name, .. }
            | Self::Channel { name, .. }
            | Self::Role { name, .. }
            | Self::File { name }
            | Self::OwnIdentity { name }
            | Self::Named { name } => name,
            Self::Person { fingerprint, .. } => fingerprint,
            Self::Link { host } => host,
        }
    }
}

/// A fact the review states. Typed, so each frontend words it in its own
/// idiom and all state the same facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fact {
    /// The subject is not told.
    SubjectNotTold,
    /// Pending messages from the subject are dropped.
    PendingMessagesDropped,
    /// Our secure session with the subject is deleted.
    SessionDeleted,
    /// Our secure session with the subject stays until the new one their
    /// answer opens replaces it (a reset we start).
    SessionKeptUntilReplaced,
    /// The messages from the subject that could not be opened after our
    /// retry requests are lost unless they send them again (a re-initiation
    /// after D24's retry requests fail, ruling Q1).
    UnopenedMessagesLost,
    /// The key fingerprint to compare out of band.
    #[serde(rename_all = "camelCase")]
    Fingerprint { fingerprint: String },
    /// The safety number to compare out of band.
    #[serde(rename_all = "camelCase")]
    SafetyNumber { safety_number: String },
    /// The subject's identity key changes: the safety number now and the
    /// one with their new key, to compare out of band (threat model S1).
    #[serde(rename_all = "camelCase")]
    SafetyNumberChanged {
        old_safety_number: String,
        new_safety_number: String,
    },
    /// This many members are affected.
    #[serde(rename_all = "camelCase")]
    MembersAffected { count: u32 },
    /// This many messages are deleted for everyone.
    #[serde(rename_all = "camelCase")]
    MessagesDeleted { count: u32 },
    /// The subject can see our presence and reach us.
    PresenceVisible,
    /// Our IP address reaches a third party.
    IpAddressLeaves,
    /// Rejoining needs a new invite.
    RejoinNeedsInvite,
}

/// Whether the action can be taken back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reversibility {
    /// An undo is offered right after.
    Undoable,
    /// Another action reverses it.
    ReversibleByInverse(IntentKind),
    /// Reversing it needs others (a peer's consent, a new invite).
    NeedsOthers,
    Never,
}

/// Who learns of the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Notifies {
    Nobody,
    Subject,
    Members(u32),
}

/// How an action that ran can be taken back for a short while (plan E1.1):
/// `IpcRequest::Undo { id }` runs the inverse the host stored, classified on
/// its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoOffer {
    pub id: IntentId,
    /// The action the undo runs.
    pub inverse: IntentKind,
    /// After this the offer is gone, ms since the epoch.
    pub expires_at_ms: u64,
}

impl UndoOffer {
    /// Whole seconds left at `now_ms` (0 once it expired).
    #[must_use]
    pub fn seconds_left(&self, now_ms: u64) -> u64 {
        self.expires_at_ms.saturating_sub(now_ms) / 1000
    }
}

/// A held action, as every frontend renders it (step 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentReview {
    /// The commit's idempotency key.
    pub intent_id: IntentId,
    pub kind: IntentKind,
    pub class: IntentClass,
    pub subject: Subject,
    pub facts: Vec<Fact>,
    pub escalations: Vec<Escalation>,
    pub reversibility: Reversibility,
    pub notifies: Notifies,
    pub proof: ProofRequired,
    pub expires_at_ms: u64,
    /// Cancelling it runs an action of its own (a backend hold's answer,
    /// such as the decline a session-reset request gets): only the
    /// review's explicit Cancel control cancels it, never Esc, a click
    /// outside or closing the view.
    pub cancel_sends: bool,
}

/// The proof a commit carries (step 3).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntentProof {
    Commit,
    Acknowledged,
    Typed(String),
    Passphrase(Zeroizing<String>),
}

impl std::fmt::Debug for IntentProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Commit => f.write_str("Commit"),
            Self::Acknowledged => f.write_str("Acknowledged"),
            Self::Typed(text) => f.debug_tuple("Typed").field(text).finish(),
            Self::Passphrase(_) => f.write_str("Passphrase(<redacted>)"),
        }
    }
}

impl IntentProof {
    /// Whether this proof has the shape `required` asks for, with a typed
    /// name matching `subject`. A passphrase is checked by the host against
    /// the vault, not here.
    #[must_use]
    pub fn satisfies(&self, required: ProofRequired, subject: &Subject) -> bool {
        match (required, self) {
            (ProofRequired::None, _)
            | (ProofRequired::Commit, Self::Commit)
            | (ProofRequired::Acknowledge, Self::Acknowledged)
            | (ProofRequired::Passphrase, Self::Passphrase(_)) => true,
            (ProofRequired::TypeSubject, Self::Typed(text)) => text == subject.typed_name(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
