//! v2.0 Governance entry types for flat SMPL CRDT.
//!
//! Each member writes GovernanceEntry variants to their own subkey in the
//! governance SMPL record (o_cnt: 0). All entries carry a Lamport timestamp
//! for deterministic CRDT merge ordering.
//!
//! See architecture doc §4.3 Record 1 and §4.4 for merge rules.
//! See rekindle-architecture-v2.md §4 for field specifications.

mod entry;
mod onboarding;
mod payload;

#[cfg(test)]
mod tests;

pub use entry::{AdmissionMode, GovernanceEntry};
pub use onboarding::{GuideStep, OnboardingOption, OnboardingQuestion, WelcomeChannel};
pub use payload::GovernanceSubkeyPayload;
