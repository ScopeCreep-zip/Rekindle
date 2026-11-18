//! Community onboarding answers (architecture §28.4) and the role change
//! they announce: the pure answer validation lives here; AppState / SQLite / sync I/O stays behind
//! `MembershipEventDeps` (`deps.rs`), implemented by the Host's
//! `HostGovernance`.

pub mod deps;
pub mod onboarding;
pub mod roles_changed;

pub use deps::MembershipEventDeps;
pub use onboarding::process_onboarding_answers;
pub use roles_changed::process_member_roles_changed;
