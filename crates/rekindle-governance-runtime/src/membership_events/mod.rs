//! Phase 23.E — legacy community membership-event handlers.
//!
//! These interpret incoming `ControlPayload` variants (JoinAccepted,
//! keypair grants, role changes, onboarding answers, peer-assisted join)
//! drained out of `src-tauri/src/services/veilid/legacy/`. The pure
//! decision logic (MEK generation matching, answer→role validation,
//! keypair unwrap/derive) lives here; AppState / SQLite / Stronghold /
//! Veilid I/O stays behind `MembershipEventDeps` (`deps.rs`), implemented
//! by the src-tauri `GovernanceAdapter`.

pub mod deps;
pub mod grants;
pub mod join_accepted;
pub mod mek_decrypt;
pub mod onboarding;
pub mod peer_join;
pub mod roles_changed;

pub use deps::{MemberUpsertRow, MembershipEventDeps, SlotGrantUpdate};
pub use grants::{process_admin_keypair_grant, process_slot_keypair_grant};
pub use join_accepted::{process_join_accepted, JoinAcceptedInput};
pub use mek_decrypt::{decrypt_with_cached_mek, MekDecryptResult};
pub use onboarding::process_onboarding_answers;
pub use peer_join::process_peer_assisted_join;
pub use roles_changed::process_member_roles_changed;
