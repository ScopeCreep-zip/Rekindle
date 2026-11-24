//! Community onboarding answers (architecture §28.4): the role change they
//! make is announced from the merged state's diff by the Host. The pure answer validation lives here; AppState / SQLite / sync I/O stays behind
//! `MembershipEventDeps` (`deps.rs`), implemented by the Host's
//! `HostGovernance`.

pub mod deps;
pub mod onboarding;

pub use deps::MembershipEventDeps;
pub use onboarding::process_onboarding_answers;
