//! Errors for Phase 18 community lifecycle ops.
//!
//! Each variant maps to a failure mode in apply / origin / bootstrap /
//! join / segments. The `Adapter` variant wraps src-tauri adapter
//! failures (Veilid attach loss, Stronghold I/O, SQL failure) as opaque
//! strings — the crate never types these directly.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GovernanceRuntimeError {
    /// The community's Lamport clock could not produce a timestamp.
    #[error(transparent)]
    Lamport(#[from] rekindle_types::lamport::LamportError),

    #[error("community not found: {0}")]
    CommunityNotFound(String),

    #[error("governance state not loaded for community {0}")]
    GovernanceStateMissing(String),

    #[error("identity secret not available")]
    IdentitySecretUnavailable,

    #[error("not attached to Veilid")]
    NotAttached,

    #[error("no pseudonym key for community {0}")]
    PseudonymKeyMissing(String),

    #[error("invalid pseudonym hex: {0}")]
    InvalidPseudonymHex(String),

    #[error("no governance key for community {0}")]
    GovernanceKeyMissing(String),

    #[error("no slot index for community {0}")]
    SlotIndexMissing(String),

    #[error("no slot keypair for community {0}")]
    SlotKeypairMissing(String),

    #[error("no slot seed for community {0}")]
    SlotSeedMissing(String),

    #[error("insufficient permission for this governance operation")]
    PermissionDenied,
    /// No `GenesisCert` is pinned for the community (S5 writes it at
    /// origin, S6 pins it at join).
    #[error("no genesis certificate is pinned for community {0}")]
    GenesisCertMissing(String),
    /// An op's author has no member record in the merged state.
    #[error("no member record is known for {0}")]
    MemberRecordMissing(String),
    /// Our own op failed the verification every reader runs.
    #[error("our governance op was refused: {0}")]
    OpRefused(String),

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

    #[error("crypto/serialization error: {0}")]
    Crypto(String),

    #[error("encoding error: {0}")]
    Encoding(String),

    #[error("adapter error: {0}")]
    Adapter(String),

    /// The inviter answered our `BootstrapRequest` and refused it: its merge
    /// does not hold our `MemberJoined` (plan E3.2). Distinct from an
    /// inviter that is not reachable, where the join stands from the DHT.
    #[error("the inviter refused the join: {0}")]
    BootstrapRefused(String),

    /// The inviter answered `bootstrapPartial` and none of the members
    /// asked after it (this many) answered a frontier: each was partial,
    /// unreachable or refused (plan E3.2 S7c.8 (i),
    /// `crate::bootstrap_peers`). The join stands from the DHT.
    #[error("the inviter is partial and none of the {0} other members asked answered")]
    BootstrapUnavailable(usize),

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
