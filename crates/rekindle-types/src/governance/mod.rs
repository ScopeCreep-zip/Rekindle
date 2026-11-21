//! v2.0 Governance entry types for flat SMPL CRDT.
//!
//! Each member writes GovernanceEntry variants to their own subkey in the
//! governance SMPL record (o_cnt: 0). Entries carry no clock: the op DAG orders them
//! for deterministic CRDT merge ordering.
//!
//! See architecture doc §4.3 Record 1 and §4.4 for merge rules.
//! See rekindle-architecture-v2.md §4 for field specifications.

mod checkpoint;
mod device;
mod entry;
mod mod_report;
mod onboarding;
mod op;
mod payload;
mod role_edit;
mod successor;

#[cfg(test)]
mod tests;

mod automod;
pub use automod::AutoModAction;
pub use checkpoint::Checkpoint;
pub use device::{
    DeviceEndorsement, DeviceReadmit, DeviceRetired, DeviceSetElement, DeviceSetElementKind,
    MAX_DEVICE_SET_ELEMENTS,
};
pub use entry::{AdmissionMode, GovernanceEntry, SlotHead};
pub use mod_report::ModReportTarget;
pub use onboarding::{
    GuideStep, OnboardingOption, OnboardingQuestion, WelcomeChannel, MAX_ONBOARDING_GUIDE_STEPS,
    MAX_ONBOARDING_ID_CHARS, MAX_ONBOARDING_OPTIONS_PER_QUESTION, MAX_ONBOARDING_QUESTIONS,
    MAX_ONBOARDING_TITLE_CHARS, MAX_ONBOARDING_WELCOME_CHARS, MAX_WELCOME_SCREEN_CHANNELS,
};
pub use op::{
    Frontier, GenesisCert, GovernanceOp, InviteProof, OpHash, OpRef, RosterChecksum,
    MAX_ADDITIONAL_CREATORS,
};
pub use payload::GovernanceSubkeyPayload;
pub use role_edit::ExclusionGroupEdit;
pub use successor::{CommunitySuccessor, CreatorLink, SuccessorReason, SuccessorShape};
