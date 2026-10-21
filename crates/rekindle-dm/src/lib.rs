//! `rekindle-dm` — group DMs (plan E2.6; architecture §27.2): 3-8 members,
//! no governance, sender-owned keys.
//!
//! Each member holds a random group sending key of its own and sends it to
//! every other member over their pairwise session; the creator's signed
//! `GroupDmInvite` is the roster. Content is sealed under the sender's key
//! and written to the sender's group chain. On any leave or block every
//! remaining member rotates its own key. A sender on the signed roster of
//! a group we accepted is admitted for the group kinds only
//! ([`admission`]).
//!
//! The crate is pure: the host's side is [`GroupDmDeps`] (its impl is the
//! Host adapter in `rekindle-node`), the crypto is `rekindle-secrets`'s,
//! and nothing here links Veilid or the transport. [`video`] is the 1:1 DM
//! video reassembly buffer (until E4.3.8).

pub mod admission;
pub mod error;
pub mod group;
pub mod lifecycle;
pub mod ports;
pub mod sender_key;
pub mod video;

pub use admission::{admit, AcceptedGroup, Admission, GroupFrame};
pub use error::DmError;
pub use group::{build_invite, verify_invite, KeyUpdate, Roster};
pub use lifecycle::{
    accept, create, decline, leave, on_decline, on_invite, on_member_key, on_member_left,
    open_received, remove_member, rotate_own_key, seal_for_send, Created, Outbound,
};
pub use ports::{GroupDmDeps, GroupState, PendingInvite};
pub use sender_key::GroupSenderKey;
pub use video::{AssembledFrame, DmVideoFragment, DmVideoReassemblyState, FRAGMENT_PAYLOAD_LIMIT};
