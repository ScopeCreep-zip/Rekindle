//! Errors for Phase 18 community lifecycle ops.
//!
//! Each variant maps to a failure mode in apply / origin / bootstrap /
//! join / segments. The `Adapter` variant wraps src-tauri adapter
//! failures (Veilid attach loss, Stronghold I/O, SQL failure) as opaque
//! strings — the crate never types these directly.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GovernanceRuntimeError {
    #[error("community not found: {0}")]
    CommunityNotFound(String),

    #[error("governance state not loaded for community {0}")]
    GovernanceStateMissing(String),

    #[error("identity secret not available")]
    IdentitySecretUnavailable,

    /// This device's op key for the community is gone from its vault
    /// while its endorsement stands (ADR 0022 rule 4): refused, never
    /// minted again.
    #[error("this device holds no op key for community {0}")]
    DeviceOpKeyMissing(String),

    /// The merged device set holds no endorsement of this device's op key
    /// in the community, so no record is ours to write its ops to (ADR
    /// 0022 rule 3): refused, never written to `M(p)` instead.
    #[error("this device's op key is not endorsed in community {0}")]
    DeviceUnendorsed(String),

    #[error("not attached to Veilid")]
    NotAttached,

    /// A create did not finish within [`crate::origin::CREATE_DEADLINE`];
    /// everything it made was released and nothing was kept.
    #[error(
        "the network did not take the new community within {0} s; nothing was created, try again"
    )]
    CreateTimedOut(u64),

    #[error("no pseudonym key for community {0}")]
    PseudonymKeyMissing(String),

    #[error("invalid pseudonym hex: {0}")]
    InvalidPseudonymHex(String),

    #[error("no anchor key for community {0}")]
    AnchorMissing(String),

    #[error("no slot index for community {0}")]
    SlotIndexMissing(String),

    #[error("no slot keypair for community {0}")]
    SlotKeypairMissing(String),

    #[error("no slot seed for community {0}")]
    SlotSeedMissing(String),

    #[error("insufficient permission for this governance operation")]
    PermissionDenied,

    /// The Host's reason for refusing a permission check.
    #[error("{0}")]
    PermissionRequired(String),

    /// The Host refused the forum starter during the atomic write.
    #[error("{0}")]
    ForumStarterRefused(String),

    #[error("membership ended while preparing the forum starter")]
    ForumStarterRemoved { banned: bool },
    /// No `GenesisCert` is pinned for the community (S5 writes it at
    /// origin, S6 pins it at join).
    #[error("no genesis certificate is pinned for community {0}")]
    GenesisCertMissing(String),
    /// An op's author has no member record in the merged state.
    #[error("no member record is known for {0}")]
    MemberRecordMissing(String),
    /// The unlock barrier found our device key ahead of this copy of the
    /// data root (plan E3.2 S7d-keys.8): an older copy was restored, and
    /// nothing is signed under the key again.
    #[error("{community}: this device's key signed seq {last_seq} from another copy of this data")]
    RestoreDetected { community: String, last_seq: u64 },
    /// The merged device set holds a `DeviceRetired` of this device's own
    /// op key, retired from seq `retired_from` by another copy of this data
    /// root or by the user (ADR 0022 rule 4, plan E3.2 S7d-keys.9): this
    /// device stops signing under it. The text is the notice the plan words,
    /// shown as the write's refusal (a held write's through
    /// `GovernanceRuntimeEvent::HeldWriteRefused`).
    #[error("this device's key was retired by another copy of this data")]
    OwnKeyRetired {
        community: String,
        retired_from: u64,
    },
    /// A write whose answer needs the op signed (an invite link, which is
    /// handed out only once it landed) while the unlock barrier is still
    /// catching up (plan E3.2 S7d-keys.8): refused at once, nothing signed. The
    /// text is the window's reason until the denied-capability view
    /// (CAPR, `CommunityCapability::CreateInvite`) carries it.
    #[error(
        "This community is catching up; invites resume once a member confirms where this device left off."
    )]
    CatchingUp(String),
    /// Such a write past the barrier waited in line behind our own earlier
    /// ops, and its turn did not come by its command's network deadline
    /// (plan E3.2 S7d-keys.8, lead ruling 2026-11-24): refused, nothing
    /// signed; the earlier ops still land.
    #[error("Your earlier change is still being published; try again in a moment.")]
    OwnOpsPending(String),
    /// An invite's steps before signing (its secrets record, the state
    /// read under the rebuild lock) did not finish within its command's
    /// network deadline (`NETWORK_ANSWER_DEADLINE`): nothing was signed and
    /// no link handed out.
    #[error("The network did not answer within {0} s; no invite was made, try again.")]
    InviteTimedOut(u64),
    /// Our own op failed the verification every reader runs.
    #[error("our governance op was refused: {0}")]
    OpRefused(String),
    /// A device-key act (ADR 0022) that cannot be written from the held
    /// set: why, in words.
    #[error("device key act refused: {0}")]
    DeviceActRefused(&'static str),

    /// The entry carries a value outside its reader-validated bound
    /// (`rekindle_governance::validate::bounds`, plan E3.0): every reader
    /// would drop it, so it is never written. `field` names the entry field.
    #[error("governance entry out of bounds: {field}")]
    OutOfBounds { field: &'static str },

    /// The entry breaks a structural invariant every reader checks
    /// (`rekindle_governance::validate::well_formed`), such as an automod
    /// regex that does not compile: no peer applies it, so it is never
    /// written.
    #[error("governance entry malformed: every reader drops it")]
    Malformed,

    #[error("governance write conflicted with newer network state ({0} bytes)")]
    WriteConflict(usize),

    #[error("governance verify failed: read-back differs ({read} bytes vs our {written})")]
    VerifyMismatch { read: usize, written: usize },

    #[error("governance verify read-back returned empty after write")]
    VerifyEmpty,

    /// Our own primary governance subkey is present on the DHT but its payload
    /// fails W26 verification (corruption, or a legacy signature from an
    /// incompatible format). Refuse to write rather than overwrite the slot with
    /// only the new entry, which would destroy our genesis/governance. Distinct
    /// from a genuinely empty slot, where a fresh write is safe.
    #[error(
        "governance primary subkey {slot} is present but unverifiable — refusing to overwrite"
    )]
    PrimarySubkeyUnverifiable { slot: u32 },

    /// Safety net under `GovernanceOverflow` paging: a single governance entry
    /// too large to fit even a whole overflow page (real entries are < ~500 B).
    /// Accumulation no longer reaches here — past one page an author's writes
    /// spill into member-owned overflow records (see `overflow`).
    #[error("governance entry too large: {bytes} B exceeds the per-page cap of {cap} B")]
    SubkeyOverflow { bytes: usize, cap: usize },

    #[error("segment cap reached ({0}); raise MAX_SEGMENTS once lazy-fetch lands")]
    SegmentCapReached(u32),

    #[error("current segment still has open slots — expansion is only allowed when full")]
    SegmentNotFull,

    #[error("crypto/serialization error: {0}")]
    Crypto(String),

    #[error("encoding error: {0}")]
    Encoding(String),

    #[error("adapter error: {0}")]
    Adapter(String),
    /// A read marker names no message: it is always a stored one's id
    /// (plan E3.1 G1, finding IB2).
    #[error("a read marker names no message")]
    NoMessageId,

    /// The inviter answered our `BootstrapRequest` and refused it: its merge
    /// does not hold our `MemberJoined` (plan E3.2). Distinct from an
    /// inviter that is not reachable, where the join stands from the DHT.
    #[error("the inviter refused the join: {0}")]
    BootstrapRefused(rekindle_types::join_refusal::JoinRefusal),
    /// Our merge does not admit a joiner's `MemberJoined`, and why: the
    /// answer to its `BootstrapRequest` is `bootstrapRefused`.
    #[error("the merge does not admit the joiner: {0}")]
    JoinNotAdmitted(rekindle_types::join_refusal::JoinRefusal),
    /// A joiner's `MemberJoined` waits on `missing` deps (its causal past)
    /// we neither hold nor could fetch yet, so no merge of ours can judge
    /// it (plan E3.2 JN1). Not a refusal: the answer to its
    /// `BootstrapRequest` is `RETRY_LATER`, and nothing is stored.
    #[error("the joiner's op waits on {missing} deps not fetchable yet: retry later")]
    JoinUnresolved { missing: usize },
    /// Our own join's walk of the op DAG holds the invite not live, and
    /// why (`join_flow::check_invite`): the reason the inviter's merge
    /// would answer, so the join is refused the same either way.
    #[error("the invite is not live: {0}")]
    InviteRefused(rekindle_types::join_refusal::JoinRefusal),
    /// Our own join's walk could not fetch an op the community's history
    /// cites (the first one named: its author's member record and seq), so
    /// no merge of what was walked is the community's state and nothing is
    /// judged from it (`join_flow::refuse_incomplete`). Not a refusal of
    /// the invite: the join may be tried again once the op is on the
    /// network.
    #[error(
        "This community's history is incomplete on the network (the op at seq {author_seq} of member record {author_record} cannot be fetched); try again later."
    )]
    HistoryIncomplete {
        author_record: String,
        author_seq: u64,
    },

    /// The inviter answered `bootstrapPartial` and none of the members
    /// asked after it (this many) answered a frontier: each was partial,
    /// unreachable or refused (plan E3.2 S7c.8 (i),
    /// `crate::bootstrap_peers`). The join stands from the DHT.
    #[error("the inviter is partial and none of the {0} other members asked answered")]
    BootstrapUnavailable(usize),

    /// The peer answered `RETRY_LATER`
    /// (`rekindle_types::message::CALL_RETRY_LATER`): it could not judge our
    /// call within its deadline (review RV5 H1). Not a refusal: the call is
    /// attempted again as after a timeout (the join's bootstrap: a
    /// `bootstrap` outbox row).
    #[error("the peer could not answer in time: retry later")]
    PeerRetryLater,

    /// Our op did not land on our member record `M` within the command's
    /// deadline; it stays queued and the outbox lands it later (plan E3.2
    /// S6b).
    #[error("not yet published: the op is queued until it lands")]
    NotYetPublished,

    /// We hold no checkpoint with the asked height and digest, or our merge
    /// of its cut does not reproduce its `stateRoot`, or the asked offset is
    /// past the snapshot (plan E3.2 S7c.3): no snapshot is served.
    #[error("no checkpoint state to serve: {0}")]
    NoCheckpointState(String),

    /// The community was joined from a checkpoint hint and its backfill has
    /// not ended (plan E3.2 S7c.8 (1), Synapse `is_partial_state_room`): it
    /// answers no hint, snapshot or bootstrap request until it holds every
    /// op.
    #[error("community {0} is partial until its backfill ends")]
    PartialState(String),
}

impl GovernanceRuntimeError {
    /// Convenience constructor for src-tauri adapter impls so they can
    /// surface tauri/veilid/sqlite errors through one variant without
    /// the crate having to know about those types.
    pub fn adapter(msg: impl Into<String>) -> Self {
        Self::Adapter(msg.into())
    }
}

/// An op with no canonical form is never signed or published: the codec's
/// refusal is an encoding error (`op_log::build_op`).
impl From<rekindle_codec::CodecError> for GovernanceRuntimeError {
    fn from(e: rekindle_codec::CodecError) -> Self {
        Self::Encoding(e.to_string())
    }
}
