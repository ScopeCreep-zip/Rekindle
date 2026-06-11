//! Delegation grants — first-class signed edges between equal peers.
//!
//! Authority is expressed through `DelegationGrant` wire objects, not
//! through identity kind. `CapabilitySet` is the lattice; delegation
//! attenuates monotonically via intersection; chain depth is bounded.

pub mod capability;
pub mod delegation;
pub mod chain;

pub use capability::{Capability, CapabilitySet, CustomCapability};
pub use delegation::{DelegationGrant, GrantScope, GRANT_CHAIN_MAX_DEPTH};
pub use chain::{verify_chain, EffectiveAuthority, EdgeEpochOracle, AcceptAllEpochs};
